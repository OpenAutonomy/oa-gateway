# Writing an adapter

An adapter owns the protocol on one side of the gateway: the socket, framing, handshake, and any schema translation. The engine owns routing and nothing else. A new protocol is a new adapter; it is not a change to `oa-gateway-core`. The crate graph and an example path are in [architecture.md](architecture.md).

A minimal example is in the `oa-gateway-adapter` crate docs (`cargo doc -p oa-gateway-adapter --open`, or `crates/oa-gateway-adapter/src/lib.rs`). That example is a doc-test, so CI fails if it stops compiling. `crates/oa-gateway-adapter/tests/echo.rs` runs the same adapter under traffic: a Ping on `demo` must produce a Pong.

## Contract

```rust
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    fn id(&self) -> &AdapterId;
    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError>;
}
```

`run` is the adapter's lifetime. Accept connections, read frames, publish envelopes, and return when `shutdown` fires. Returning `Err` is fatal for that adapter only. The host logs the error and leaves the others running.

The design depends on four rules:

1. **Do not parse a payload in the core, and do not name a protocol there.** If `oa-gateway-core` would need to know what the bytes mean, the logic belongs in the adapter crate.
2. **Do not call another adapter.** Adapters share an `Engine` and nothing else. Two adapters exchange data by publishing and subscribing, which is what makes them independently testable and removable.
3. **Own the delivery channel.** Create the `mpsc::Sender` passed to `subscribe`, and read the matching receiver.
4. **Leave the engine clean.** Call `engine.drop_adapter(id)` on stop. Otherwise the subscriptions keep matching and silently discard messages.

## Routing

A [`RouteKey`](../crates/oa-gateway-core/src/route.rs) is a `topic` plus an optional `type_hint`:

- `RouteKey::typed("PositionReport", "PositionReport")` matches one message type on one topic.
- `RouteKey::topic("PositionReport")` matches every type on that topic. A subscription with `type_hint: None` is a wildcard.

`type_hint` is whatever discriminator the protocol has: an OWP message name, a UCI message type, or a PDU type. The engine compares it for equality and does not interpret it.

Publishing is fan-out to matching subscribers. A publish with `type_hint: Some("Ping")` reaches both `typed(topic, "Ping")` and `topic(topic)` subscribers.

## Envelopes and headers

An [`Envelope`](../crates/oa-gateway-core/src/envelope.rs) carries an id, a route, string headers, a content-type label, and an opaque `Bytes` payload. The engine reads only the route.

Headers are namespaced by owner so envelopes stay legible as they cross adapters:

| Prefix | Owner | Examples |
|---|---|---|
| `oag.` | gateway-wide | `oag.origin_adapter`, `oag.topic`, `oag.type_hint`, `oag.id` |
| `stomp.` | STOMP adapter | `stomp.destination`, `stomp.message-id` |
| `agra.` | A-GRA wrappers | `agra.wrapper`, `agra.command_id`, `agra.originator_uuid` |

The `oag.*` constants live in `oa-gateway-core` (`HDR_ORIGIN`, `HDR_TOPIC`, `HDR_TYPE_HINT`, `HDR_ID`). `oa-gateway-stomp` re-exports them. Use [`Envelope::with_origin`](../crates/oa-gateway-core/src/envelope.rs) and [`Envelope::is_echo_of`](../crates/oa-gateway-core/src/envelope.rs) rather than repeating the strings.

## Echo suppression

An adapter that bridges an external bus must not send back what it just received, or a message loops between the gateway and the broker. The convention is two-sided:

- On the way in, stamp with `envelope.with_origin(&your_id)`.
- On the way out, skip when `envelope.is_echo_of(&your_id)`.

The engine does not skip by origin. One adapter id may cover many connections, as OWP does. STOMP implements the convention in `inbound_publish` and `forward_outbound`. DDS does the same, and the rustdds provider also drops samples whose writer shares this participant's GUID prefix, because rustdds delivers local writes. `[stomp] suppress_echo` and `[dds] suppress_echo` (both default `true`) are the knobs. A new bridging adapter must do the same.

## Backpressure

`Engine::publish` uses `try_send`, so a full subscriber loses the message instead of blocking the publisher. Channel capacity defaults to `DEFAULT_CHANNEL_CAPACITY` (64). If the adapter can be slower than the traffic it subscribes to, read the receiver on a dedicated task and buffer on the adapter's own terms.

Drops are counted in `EngineStats`. The host logs those counters on `[engine] stats_interval_secs` (default 30; `0` disables) and warns when `dropped` increased. Publish sites that inspect `PublishOutcome` also log a per-message drop.

## Lifecycle

The adapter sequences its own startup, teardown, and reconnect:

- **Subscribe after the transport is up**, so deliveries are not queued before they can be sent.
- **Select on `shutdown.cancelled()`** in the same loop that reads the transport, so cancellation is observed promptly.
- **Put the retry loop outside the session.** `StompAdapter::serve_inner`, `OwpAdapter::run`, and `DdsAdapter::run` each run one session on a child task, so a panic there is a join error rather than an unwind that would take the retry loop down with it — the join result is fed to the shared `oa_gateway_adapter::after_join`. `on_panic` is `abort` (default: a panic ends `run`) or `reconnect` (treat the panic as a failed session, then follow the adapter's own `reconnect` setting). The host does not restart a finished `run`. Loopback has no session and skips this entirely — nothing in it can fail or panic in normal operation.
- **Call `drop_adapter` on the way out, and again when a session restarts.** The STOMP adapter also calls it at session start, which clears stale subscriptions left by a previous connection.

## Host wiring

The host crate (`crates/oa-gateway`) reads a TOML section per adapter (`src/config/`), validates it, and spawns `run` (`src/adapters/`). Naming the table starts the adapter. `enabled` must default to `true` when the section is present and to `false` in `Default` (the omitted-section path).

To add an adapter, add a `#[derive(Deserialize)]` section struct in `src/config/` with `#[serde(deny_unknown_fields)]` and `#[serde(default = "...")]` on every field, resolve addresses with `resolve_addr` before spawning when the protocol has a hostname, and start the adapter from `src/adapters/`. Add the section to a shipped example. DDS has no hostname; it checks that `[dds] qos` exists instead. The `shipped_configs_parse` test fails if a shipped file and the struct disagree.

## Testing

Adapter crates keep unit tests for their own codec and mapping logic. Traffic that crosses the engine belongs in `oa-gateway-testing`. Its `harness` feature provides OWP, STOMP, and DDS helpers and `start_mini_broker()`, an in-process STOMP broker on an ephemeral port, so most tests do not need Docker.

The fastest end-to-end check is against `Loopback`: subscribe a loopback handle to the topic the adapter publishes, drive the adapter's transport, and assert on the envelope that arrives. `crates/oa-gateway-testing/tests/stomp_bridge.rs` and `tests/dds_bridge.rs` are the pattern to copy.

## Reference adapters

Read them in this order:

1. `oa-gateway-loopback` — the smallest complete adapter, about 90 lines including the trait implementation.
2. `oa-gateway-stomp` — a client bridge: framing, handshake, reconnect, echo suppression, and destination mapping.
3. `oa-gateway-dds` — a domain participant: provider shim, file-based QoS, A-GRA samples, and echo skip including local rustdds writes.
4. `oa-gateway-owp` — a server: it accepts connections, tracks per-connection subscriptions, and performs schema translation.

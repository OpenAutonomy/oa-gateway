# Writing an adapter

An adapter owns one side of the gateway: its socket, its framing, its handshake, and any schema translation. The engine owns routing and nothing else. Adding a protocol means adding an adapter; it should never mean changing `mpg-core`.

A runnable minimal adapter lives in the `mpg-adapter` crate docs (`cargo doc -p mpg-adapter --open`, or read `crates/mpg-adapter/src/lib.rs`). It is a doc-test, so CI fails if it stops compiling or stops working.

## The contract

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

`run` is your whole lifetime. Accept connections, read frames, publish envelopes, and return when `shutdown` fires. Returning `Err` is fatal for your adapter only — the host logs it and leaves the others running.

Four rules carry the design:

1. **Never parse a payload in the core, and never name a protocol there.** If `mpg-core` would need to know what your bytes mean, the logic belongs in your crate.
2. **Never talk to another adapter.** Adapters share an `Engine` and nothing else. Two adapters exchanging data do it by publishing and subscribing, which is what makes them independently testable and removable.
3. **Own your channels.** You create the `mpsc::Sender` you hand to `subscribe`, and you read the matching receiver.
4. **Leave the engine clean.** Call `engine.drop_adapter(id)` when you stop, or your subscriptions keep matching and silently discarding messages.

## Routing

A [`RouteKey`](../crates/mpg-core/src/route.rs) is a `topic` plus an optional `type_hint`:

- `RouteKey::typed("PositionReport", "PositionReport")` — one message type on one topic.
- `RouteKey::topic("PositionReport")` — every type on that topic. A subscription with `type_hint: None` is a wildcard.

`type_hint` is whatever discriminator your protocol has: an OWP message name, a UCI message type, a PDU type. The engine only compares it for equality, so its meaning stays yours.

Publishing is fan-out to matching subscribers, and matching is exact-plus-wildcard: a publish carrying `type_hint: Some("Ping")` reaches both `typed(topic, "Ping")` and `topic(topic)` subscribers.

## Envelopes and headers

An [`Envelope`](../crates/mpg-core/src/envelope.rs) is an id, a route, string headers, a content-type label, and an opaque `Bytes` payload. The engine reads only the route.

Headers are namespaced by owner. Keep to the convention so envelopes stay legible as they cross adapters:

| Prefix | Owner | Examples |
|---|---|---|
| `mpg.` | gateway-wide | `mpg.origin_adapter`, `mpg.topic`, `mpg.type_hint`, `mpg.id` |
| `stomp.` | STOMP adapter | `stomp.destination`, `stomp.message-id` |
| `agra.` | A-GRA wrappers | `agra.wrapper`, `agra.command_id`, `agra.originator_uuid` |

The `mpg.*` constants currently live in `mpg-stomp`'s `dest` module and are re-exported from that crate. They are gateway-wide by intent rather than by placement, so expect them to move to `mpg-core` — import them rather than retyping the strings.

## Avoiding echo loops

Any adapter bridging an external bus must not send back what it just received, or a message loops forever between the gateway and the broker. The convention is two-sided:

- On the way in, stamp the envelope with `mpg.origin_adapter = your id`.
- On the way out, skip any delivery whose `mpg.origin_adapter` already equals your id.

`crates/mpg-stomp/src/adapter.rs` implements exactly this in `inbound_publish` and `forward_outbound`. Note that today only the STOMP adapter does — this is a convention a new bridging adapter must opt into, not something the engine enforces.

## Backpressure

`Engine::publish` uses `try_send`, so a subscriber that is full loses the message rather than blocking the publisher. Channel capacity defaults to `DEFAULT_CHANNEL_CAPACITY` (64). If your adapter can be slower than the traffic it subscribes to, read the receiver in a dedicated task and buffer on your own terms.

Drops are counted in `EngineStats` but nothing currently reports them, so a slow adapter loses traffic quietly. Treat that as a known gap rather than a guarantee of delivery.

## Lifecycle

Startup, teardown, and reconnect are all yours to sequence:

- **Subscribe after your transport is up**, so you are not accumulating deliveries you cannot yet send.
- **Select on `shutdown.cancelled()`** in the same loop that reads your transport, so cancellation is observed promptly.
- **Put the retry loop outside the session.** `StompAdapter::serve_inner` loops over `session`, so one failed connection retries without losing the adapter. Note the current caveat: a panic inside the task destroys the retry loop with it, and the host does not restart adapters.
- **Call `drop_adapter` on the way out, and also when a session restarts.** The STOMP adapter calls it at session start too, which clears stale subscriptions left by a previous connection.

## Wiring into the host

`crates/mpg/src/main.rs` reads a TOML section per adapter, validates it, and spawns `run`. To add yours: define a `#[derive(Deserialize)]` section struct with `#[serde(deny_unknown_fields)]` and `#[serde(default = "...")]` on every field, resolve any addresses with `resolve_addr` before spawning anything, then push the spawned task onto `handles`. Add the section to `config/default.toml`; the `shipped_configs_parse` test will fail if the file and the struct disagree.

## Testing

Adapter crates keep unit tests for their own codec and mapping logic. Anything that crosses the engine belongs in `mpg-testing`, whose `harness` feature provides OWP and STOMP helpers plus `start_mini_broker()` — an in-process STOMP broker on an ephemeral port, so most tests need no Docker.

The fastest way to test a new adapter end to end is against `Loopback`: subscribe a loopback handle to the topic your adapter publishes, drive your adapter's transport, and assert on the envelope that arrives. `crates/mpg-testing/tests/stomp_bridge.rs` is the pattern to copy.

## Reference adapters

Read them in this order:

1. `mpg-loopback` — the smallest complete adapter, about 90 lines including the trait impl.
2. `mpg-stomp` — a client bridge: framing, handshake, reconnect, echo suppression, destination mapping.
3. `mpg-owp` — a server: accepts connections, tracks per-connection subscriptions, and does schema translation.

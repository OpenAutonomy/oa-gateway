# Connecting a WebSocket client

OWP (OMS WebSocket Protocol) is the one adapter a client talks to directly rather than through another broker or bus. STOMP and DDS bridge a fixed list of topics between the engine and something else; an OWP client opens a WebSocket, says `INIT`, and then `SUB`/`PUB` whatever topics and message types it wants for the life of that connection. `oa-gateway-bench`'s `ping` scenario and the quickstart in the [README](../README.md) are both OWP clients.

## Launch

```bash
./target/release/oa-gateway config/default.toml
```

`config/default.toml` serves OWP at `ws://127.0.0.1:9000/` with no TLS. Send one message through it:

```bash
cargo run -p oa-gateway-bench --release -- ping --url ws://127.0.0.1:9000/
```

That does `INIT`, `SUB Ping demo`, `PUB demo {"Ping":{"n":1}}`, waits for the `MSG` it should get back, and exits `0`. Reading [`crates/oa-gateway-bench/src/scenarios/ping.rs`](../crates/oa-gateway-bench/src/scenarios/ping.rs) is the fastest way to see a minimal client in Rust; [`crates/oa-gateway-testing/src/owp.rs`](../crates/oa-gateway-testing/src/owp.rs) has a second one built on plain `tokio-tungstenite`.

## Configure the adapter

Every OWP key and its default is in [configuration.md](configuration.md). The keys that shape the client-facing protocol:

```toml
[owp]
id = "owp"
bind = "127.0.0.1:9000"
server_id = "oa-gateway-0"
system_label = "OA-Gateway Prototype"
schema = "002.5.0"
unwrap_ma_payloads = true
xml_baseline = false
```

| Key | What it does |
|---|---|
| `bind` | `host:port` the WebSocket listener binds. |
| `server_id`, `system_label` | Sent back on `INFO`; identify this gateway to the client. |
| `schema` | The protocol version string `INIT.schema` must match exactly. Not a UCI XSD path — that is `[uci].schema`. Empty skips the check. |
| `unwrap_ma_payloads` | An A-GRA `MA_RxDataPayload`/`MA_TxDataPayloadCommand` wrapper on `PUB` is peeled; the engine sees the wrapper and the inner UCI message as two envelopes. |
| `xml_baseline` | Convert OMS JSON ↔ UCI XML at the socket, so a client sends and receives JSON while the engine and any bridged broker see XML. Requires `[uci].schema`. |
| `allowed_origins` | Opt-in browser `Origin` allowlist. See [Limits](#limits). |
| `init_timeout_secs`, `idle_timeout_secs` | See [Timeouts](#timeouts). |

## The protocol

A frame is one line of text: a keyword, then space- or tab-separated fields. `INIT`, `PUB`, and `INFO` keep a JSON body as the rest of the line rather than tokenizing it. Binary WebSocket frames are refused and end the session.

### Connect and INIT

Open the WebSocket with the `owp` subprotocol — the handshake is refused with `400` if `Sec-WebSocket-Protocol` does not list it, case-insensitively:

```
GET /  HTTP/1.1
Sec-WebSocket-Protocol: owp
```

The first frame from the client must be `INIT`, or the session is closed with an `Illegal-State` error:

```
INIT {"versions":["1.0"],"schema":"002.5.0","service_id":"web-app","verbose":true}
```

`versions` must include `"1.0"`; `schema` must match `[owp].schema` exactly when that key is set; `service_id` must be an OWP identifier (`^[A-Za-z0-9_\-.]+$`). `verbose` defaults to `true` and controls whether `PUB`/`SUB`/`UNSUB` get a `+OK` on success — errors are always sent regardless. A failed `INIT` (wrong version, wrong schema, bad `service_id`) gets a `-ERR` and the connection closes; a well-formed one gets `+OK` (if verbose) and then:

```
INFO {"version":"1.0","server_id":"oa-gateway-0","uuids":{"system":"…","service":"…"},"system_label":"OA-Gateway Prototype"}
```

`uuids.service` is UUIDv5 of `service_id`, so the same `service_id` always gets the same service UUID.

### Publish, subscribe, receive

```
PUB <topic> <payload>
SUB <sid> <message_name> <topic> [group]
UNSUB <sid>
```

`topic` and `sid` are OWP identifiers; `sid` is chosen by the client and must be unique per connection — reusing one before `UNSUB` is `-ERR Illegal-Argument`. `payload` on `PUB` is the rest of the line: OMS JSON or, with `xml_baseline`, UCI XML — whichever it looks like decides how it is handled, and the root key (or XML root element) becomes the route's type. `SUB` always names a `message_name`; there is no wildcard subscribe over OWP the way an engine subscription can be untyped. `group` is accepted and currently ignored.

A subscription receives:

```
MSG <sid> <payload>
```

one per matching engine delivery, for as long as the `SUB` is live. `PUB` to a topic nothing subscribes to is still `+OK` — pub/sub, not RPC — and is logged once per route rather than silently dropped:

```
WARN nothing is subscribed to this route, so the publish went nowhere route=Foo adapter=owp
```

### Errors

```
-ERR <name> [details…]
```

| `name` | When |
|---|---|
| `Unsupported-Version` | `INIT.versions` does not include `"1.0"`. |
| `Unsupported-Schema` | `INIT.schema` does not match `[owp].schema`. |
| `Unsupported-Service` | In the wire vocabulary; this build never sends it. |
| `Illegal-State` | `INIT` was not the first frame, `INIT` was sent twice, or the per-connection subscription limit was reached. |
| `Illegal-Argument` | A frame parsed but failed a semantic check — a bad identifier, a duplicate `sid`, an unknown `sid` on `UNSUB`. |
| `Illegal-Operation` | A binary WebSocket frame was sent. The session ends. |
| `Invalid-Message` | `PUB` failed to convert, unwrap, or (in `reject` mode) validate; or a delivery to this subscription failed to convert, or itself failed `reject`-mode validation, and could not be forwarded. |
| `Internal-Error` | The engine subscribe call itself failed. |

A frame the codec cannot parse at all (unknown keyword, missing field) is `Illegal-Argument` with the parse failure as `details`, and does not close the session — only a rejected `INIT`, a protocol violation after `INIT`, or a binary frame does that.

## Timeouts

A connection that never completes `INIT` is closed after `init_timeout_secs` (default 30), timed from the accepted WebSocket rather than reset by traffic — sending frames that never form a valid `INIT` does not buy more time. Once active, `idle_timeout_secs` (default 600) closes a session with no frame in either direction; any client frame and any server frame, including a delivered `MSG`, resets that clock, so a subscriber that only receives never gets closed for being idle. Either is `0` to disable. [configuration.md](configuration.md) has both keys.

## Limits

- **There is no authentication.** Any peer that completes the WebSocket handshake can `INIT` under any `service_id` and publish or subscribe to anything. [SECURITY.md](../SECURITY.md) states the assumptions; bind loopback or front this with a reverse proxy.
- **`allowed_origins` is a browser-only control, not a substitute for that proxy.** Unset, any `Origin` (including none) connects. Set, a handshake whose `Origin` is not listed verbatim — a missing `Origin` included — is refused with `403`. A non-browser client sends whatever `Origin` it likes, so this closes cross-site WebSocket access from a browser and nothing else.
- **`tls_cert`/`tls_key` make the listener speak `wss://` only**; a plaintext client is refused at the handshake. Add `tls_client_ca` and a client that cannot present a certificate from that bundle is refused too — the one form of peer authentication this gateway has, and it stops at the handshake: a verified client is not treated any differently once connected.
- **`max_frame_size`, `max_connections`, and `max_subscriptions`** bound what one peer can cost the others; a `SUB` past the limit is `-ERR Illegal-State` and the session continues, an oversized frame ends the session, and a connection past the limit is closed on accept.

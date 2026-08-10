# mpg — multi-protocol gateway (prototype)

Protocol-agnostic routing engine plus pluggable adapters. The core never parses a payload and never names a protocol. Adapters own framing, handshake, and any schema logic. Only data messages cross the engine.

Licensed under [Apache License 2.0](LICENSE) (copyright John Henry Burns). Compatible with Open Arsenal A-GRA / sleet (Apache-2.0) and MIT projects such as Ghost Detector.

```
loopback ──publish/subscribe──► Engine ◄──PUB/SUB── owp (WebSocket)
                Envelope            ▲
                                    │
                         SEND/MESSAGE (STOMP)
                                    │
                              ActiveMQ :61613
                         (Java CAL / OpenWire peers)
```

## Layout

| Crate | Role |
|---|---|
| `crates/mpg-core` | `Envelope`, `RouteKey`, `Engine` |
| `crates/mpg-adapter` | `Adapter` trait |
| `crates/mpg-agra` | A-GRA `MA_RxDataPayload` / `MA_TxDataPayloadCommand` wrap/unwrap |
| `crates/mpg-loopback` | In-process adapter |
| `crates/mpg-owp` | OWP 1.0 over WebSocket |
| `crates/mpg-stomp` | STOMP 1.2 client (ActiveMQ Classic) |
| `crates/mpg-uci` | Schema-aware UCI XML ↔ OMS JSON (2.5 slice) |
| `crates/mpg-testing` | Fixtures (always) + optional `harness` (OWP/STOMP helpers, mini broker). Cross-adapter tests live here. |
| `crates/mpg` | Host binary |

Reference clones under `repos/` are not compiled into this workspace.

## Run

```bash
cargo test --workspace --locked
cargo run -p mpg -- config/default.toml          # OWP only
cargo run -p mpg -- config/asb.toml              # + ActiveMQ STOMP (compose up first)
```

CI (GitHub Actions + GitLab CI) runs `fmt`, `clippy -D warnings`, `cargo test --workspace --locked`, then the ignored ActiveMQ XML round-trip. See [CONTRIBUTING.md](CONTRIBUTING.md).

OWP listens on `ws://127.0.0.1:9000/` with subprotocol `owp`. There is **no TLS and no authentication** — loopback bind only.

### websocat

```bash
websocat -s 0 --protocol owp ws://127.0.0.1:9000/
```

Then (local `config/default.toml`, JSON stays on the engine):

```
INIT {"versions":["1.0"],"schema":"002.5.0","service_id":"web-app","verbose":true}
SUB sub-1 PositionReport PositionReport
PUB PositionReport {"PositionReport":{"MessageData":{"n":1}}}
```

A second subscriber on another connection (or a loopback handle in tests) receives the published envelope through the engine, not through a direct adapter link.

### ActiveMQ STOMP

This is a **client** adapter, not a JMS implementation. It speaks STOMP 1.2 to a broker (ActiveMQ Classic `:61613` by default). Java CAL still uses OpenWire; ActiveMQ routes the same topic between the two protocols when the destination name matches.

Enable in `config/default.toml`:

```toml
[stomp]
enabled = true
broker = "127.0.0.1:61613"
destination_prefix = "/topic/"
topics = ["demo"]   # or PositionReport, SubsystemStatus, … for sk-cal
max_frame_size = 16777216   # optional; refuse larger frames from the broker
```

Each listed name is bridged both ways:

- engine topic `demo` ↔ STOMP `/topic/demo` (JMS topic `demo`)
- `type_hint` is sniffed from OMS JSON / XML or carried in `mpg.type_hint`
- inbound frames tagged `mpg.origin_adapter` are not echoed back to the broker

No TLS and no authentication unless you set `login` / `passcode`. Heartbeats are disabled (`0,0`). Frames over `max_frame_size` (16 MiB default) end the session and reconnect rather than growing the read buffer.

For Ghost Detector / `uci-cal-jms`, put UCI message type names in `topics` — that is what those CALs use as JMS destinations.

### Live ActiveMQ (XML `PositionReport`)

```bash
./scripts/live-activemq.sh          # compose up + ignored rust round-trip
# or: cargo test -p mpg-testing --test live_activemq -- --ignored --test-threads=1
cargo run -p mpg -- config/asb.toml # OWP :9000 + STOMP → /topic/PositionReport
```

Manual broker → mpg → OWP: `websocat` SUB `PositionReport` / `PositionReport`, then:

```bash
python3 scripts/stomp_xml_smoke.py send
```

`scripts/stomp_xml_smoke.py recv` watches the topic directly (ActiveMQ fan-out, not mpg). Outbound XML (loopback → broker) is what the ignored test asserts. Fixture XML lives in `crates/mpg-testing/fixtures/`.

No TLS. Console: <http://127.0.0.1:8161> (`admin` / `admin`).

## Internal model

- `Envelope` — id, route, string headers, content-type, opaque `Bytes` payload
- `RouteKey` — `topic` + optional `type_hint` (OWP message name today)
- Subscribe with `type_hint = None` to match every type on a topic

**Name rule (ASB path):** UCI message type = engine topic = `/topic/{type}` = JMS topic. OWP `SUB`/`PUB` use `PositionReport`, not a toy channel name.

OWP `PUB` extracts `type_hint` from the single root JSON key. With `owp.xml_baseline = true` (`config/asb.toml`), PUB converts OMS JSON → UCI XML before the engine; MSG converts XML → JSON for the OWP client. STOMP stays XML identity. Conversion uses `mpg-uci`’s hand-built 2.5 *slice* (Ping, PositionReport, PolySample `$type`, MA Rx/Tx wrappers) — not the full XSD catalog.

OWP `PUB` does **not** full-XSD-validate UCI.

### A-GRA Rx/Tx wrappers

External MA interfaces (MA-C2, MA-MA) wrap inner UCI messages as `xs:hexBinary` inside:

- `MA_RxDataPayload` — offboard → MA (Data-1)
- `MA_TxDataPayloadCommand` — MA → offboard (Command-2), with `MA_TxDataPayloadCommandStatus` ack

Platform interfaces (MA-VI, MA-MS) use native MTs and skip this envelope.

`mpg-agra` wraps and unwraps both OMS JSON and XML wrappers. With `owp.unwrap_ma_payloads = true` (default), a PUB of a wrapper fans out **two** envelopes: the wrapper MT (so Command-2 status still correlates) and the inner MT (so platform subscribers see `PositionReport`, etc.). Inner `EncodedPayload` may be hex-encoded XML or OMS JSON; `type_hint` is taken from the inner document element.

## Not in v0

Identity map, full UCI XSD load, OpenWire/JMS and DDS adapters, QoS, queue groups, TLS.

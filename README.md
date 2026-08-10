# OA-Gateway

- [Introduction](#introduction)
- [Getting started](#getting-started)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## Introduction

OA-Gateway connects systems that share a message vocabulary but not a transport. A browser client on a WebSocket and a Java CAL peer on OpenWire can exchange the same UCI message type without either knowing the other exists.

The routing core is protocol-agnostic: it never parses a payload and never names a protocol. Adapters own framing, handshake, and any schema logic; only data messages cross the engine, and control frames stay with the adapter that owns them. Supporting a new protocol means adding an adapter, not changing the core.

Message types come from the UCI 2.5 catalog (Universal Command and Control Interface), carried as OMS JSON or UCI XML. The WebSocket adapter implements OWP 1.0 as specified in OMSC-SPC-013 (Open Mission Systems). A-GRA `MA_RxDataPayload` and `MA_TxDataPayloadCommand` wrappers are peeled so platform subscribers see the inner message type. The STOMP 1.2 client bridges ActiveMQ Classic, where OpenWire CAL peers share the same JMS destinations. The gateway is built to sit alongside Open Arsenal projects such as A-GRA and sleet, and OMS Critical Abstraction Layer implementations such as sk-cal, uci-cal-jms, and Ghost Detector, rather than to replace any of them.

Goals:

- Compliance with the OMS, UCI, and A-GRA standards.
- Topic- and destination-level routing, so traffic is addressed by name rather than by peer.
- Ease of extensibility (i.e., adding new protocol adapters).

Current scope is a prototype: conversion is driven by the published UCI 2.5 XSD, which you supply locally since the standard is not redistributed here, and there is no TLS or authentication.

```
loopback ──publish/subscribe──► Engine ◄──PUB/SUB── owp (WebSocket)
                Envelope            ▲
                                    │
                         SEND/MESSAGE (STOMP)
                                    │
                              ActiveMQ :61613
                         (Java CAL / OpenWire peers)
```

<!--
## Layout

| Crate | Role |
|---|---|
| `crates/oa-gateway-core` | `Envelope`, `RouteKey`, `Engine` |
| `crates/oa-gateway-adapter` | `Adapter` trait |
| `crates/oa-gateway-agra` | A-GRA `MA_RxDataPayload` / `MA_TxDataPayloadCommand` wrap/unwrap |
| `crates/oa-gateway-loopback` | In-process adapter |
| `crates/oa-gateway-owp` | OWP 1.0 over WebSocket |
| `crates/oa-gateway-stomp` | STOMP 1.2 client (ActiveMQ Classic) |
| `crates/oa-gateway-uci` | Schema-aware UCI XML ↔ OMS JSON, compiled from the published XSD |
| `crates/oa-gateway-testing` | Fixtures (always) + optional `harness` (OWP/STOMP helpers, mini broker). Cross-adapter tests live here. |
| `crates/oa-gateway` | Host binary |

Reference clones under `repos/` are not compiled into this workspace.
-->

## Getting started

```bash
cargo test --workspace --locked
cargo run -p oa-gateway -- --help
cargo run -p oa-gateway -- config/default.toml          # OWP only, no schema needed
./scripts/fetch-uci-schema.sh                           # once, for JSON ↔ XML conversion
cargo run -p oa-gateway -- config/asb.toml              # + ActiveMQ STOMP (compose up first)
```

With no argument oa-gateway looks for `config/default.toml` in the current directory and its two parents, then falls back to built-in defaults. A config path you name explicitly must exist. Unknown keys are rejected rather than ignored, so a misspelled `topics` fails at startup instead of silently doing nothing. `owp.bind` and `stomp.broker` accept `host:port` as well as a literal address, preferring IPv4 when a name offers both.

OWP listens on `ws://127.0.0.1:9000/` with subprotocol `owp`. There is **no TLS and no authentication** — loopback bind only.

### UCI schema

Converting between OMS JSON and UCI XML needs the UCI schema. The standard is public (Distribution Statement A) but is not redistributed here, so fetch it once:

```bash
./scripts/fetch-uci-schema.sh
```

That downloads the two documents into `schema/uci/` (gitignored) and checks them against the SHA-256 digests pinned in `scripts/uci-schema.sha256`, so you always know which revision you are running. If upstream publishes a new one the checksum fails and tells you how to re-pin. `config/asb.toml` already points at that location; any other config names the files itself:

```toml
[uci]
schema = [
  "schema/uci/UCI_MessageDefinitions_v2_5_0.xsd",
  "schema/uci/UCI_SecurityMarkings_v2_5_0.xsd",
]
```

List every document the schema spans. Naming `UCI_MessageDefinitions` alone leaves the security-marking types unresolved, and startup reports the dangling names rather than letting a missing type surface later against live traffic. Compiling the published catalog takes roughly 60 ms and yields 722 message types.

Without a schema the gateway still routes: payloads cross the engine untouched and the topic stands in for the type hint. `owp.xml_baseline` exists only to convert, so enabling it without a schema is refused at startup.

The schema is read at startup only. Upgrading to a new UCI release, or narrowing to a program-specific Message Set, means pointing this at different files and restarting — no rebuild.

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
- `type_hint` is sniffed from OMS JSON / XML or carried in `oag.type_hint`
- inbound frames tagged `oag.origin_adapter` are not echoed back to the broker

No TLS and no authentication unless you set `login` / `passcode`. Heartbeats are disabled (`0,0`). Frames over `max_frame_size` (16 MiB default) end the session and reconnect rather than growing the read buffer.

For Ghost Detector / `uci-cal-jms`, put UCI message type names in `topics` — that is what those CALs use as JMS destinations.

### Live ActiveMQ (XML `PositionReport`)

```bash
./scripts/live-activemq.sh          # compose up + ignored rust round-trip
# or: cargo test -p oa-gateway-testing --test live_activemq -- --ignored --test-threads=1
cargo run -p oa-gateway -- config/asb.toml # OWP :9000 + STOMP → /topic/PositionReport
```

Manual broker → oa-gateway → OWP: `websocat` SUB `PositionReport` / `PositionReport`, then:

```bash
python3 scripts/stomp_xml_smoke.py send
```

`scripts/stomp_xml_smoke.py recv` watches the topic directly (ActiveMQ fan-out, not oa-gateway). Outbound XML (loopback → broker) is what the ignored test asserts. Fixture XML lives in `crates/oa-gateway-testing/fixtures/`.

No TLS. Console: <http://127.0.0.1:8161> (`admin` / `admin`).

## Documentation

- [docs/glossary.md](docs/glossary.md) — the acronyms, and OA-Gateway's own vocabulary
- [docs/writing-an-adapter.md](docs/writing-an-adapter.md) — adding a protocol
- `cargo doc --workspace --no-deps --open` — the crate APIs. `oa-gateway-adapter` carries a runnable minimal adapter.

<!--
## Internal model

- `Envelope` — id, route, string headers, content-type, opaque `Bytes` payload
- `RouteKey` — `topic` + optional `type_hint` (OWP message name today)
- Subscribe with `type_hint = None` to match every type on a topic

**Name rule (ASB path):** UCI message type = engine topic = `/topic/{type}` = JMS topic. OWP `SUB`/`PUB` use `PositionReport`, not a toy channel name.

OWP `PUB` extracts `type_hint` from the single root JSON key. With `owp.xml_baseline = true` (`config/asb.toml`), PUB converts OMS JSON → UCI XML before the engine; MSG converts XML → JSON for the OWP client. STOMP stays XML identity. Conversion covers whatever the schema you loaded defines, which for the published catalog is all 722 message types.

Conversion is schema-*driven*, not schema-*validating*: `PUB` does not check occurrence constraints, enumeration values, or patterns, and a field the schema does not declare is passed through rather than rejected.

### A-GRA Rx/Tx wrappers

External MA interfaces (MA-C2, MA-MA) wrap inner UCI messages as `xs:hexBinary` inside:

- `MA_RxDataPayload` — offboard → MA (Data-1)
- `MA_TxDataPayloadCommand` — MA → offboard (Command-2), with `MA_TxDataPayloadCommandStatus` ack

Platform interfaces (MA-VI, MA-MS) use native MTs and skip this envelope.

`oa-gateway-agra` wraps and unwraps both OMS JSON and XML wrappers. With `owp.unwrap_ma_payloads = true` (default), a PUB of a wrapper fans out **two** envelopes: the wrapper MT (so Command-2 status still correlates) and the inner MT (so platform subscribers see `PositionReport`, etc.). Inner `EncodedPayload` may be hex-encoded XML or OMS JSON; `type_hint` is taken from the inner document element.
-->

<!--
## Not in v0

Identity map, schema validation, OpenWire/JMS and DDS adapters, QoS, queue groups, TLS.
-->

## Contributing

CI (GitHub Actions + GitLab CI) runs `fmt`, `clippy -D warnings`, `cargo test --workspace --locked`, and `cargo doc` with warnings denied, then the ignored ActiveMQ XML round-trip. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under [Apache License 2.0](LICENSE) (copyright John Henry Burns). Compatible with Open Arsenal A-GRA / sleet (Apache-2.0) and MIT projects such as Ghost Detector.

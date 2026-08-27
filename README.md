# OA-Gateway

[![CI](https://github.com/OpenAutonomy/oa-gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/OpenAutonomy/oa-gateway/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

![OA-Gateway: multi-protocol gateway bridging ActiveMQ, DDS, and WebSockets](docs/oa-gateway-banner.png)

## Introduction

OA-Gateway is a multi-protocol message gateway written in Rust. It bridges WebSocket, ActiveMQ (STOMP), and DDS traffic through one protocol-agnostic routing core, and speaks OMS (Open Mission Systems), UCI (Universal Command and Control Interface), and A-GRA well enough to sit next to systems built on those standards — without being one of them, and without being an [Open Arsenal](https://gitlab.com/open-arsenal/) project itself.

It is an independent prototype, open source under the MIT License. That status is not just a label: there is no authentication or authorization, and TLS is off unless you configure it — see [SECURITY.md](SECURITY.md) before you point it at anything but loopback.

## What it does

- **Bridges three protocols through one router.** WebSocket (OWP), STOMP (ActiveMQ), and DDS (rustdds) adapters all publish to and subscribe from the same in-process engine. A message published on one reaches every matching subscriber on every other.
- **Routes by name, not by peer.** Traffic is addressed by topic and message type the same way across every protocol, so adding a subscriber never means telling a publisher about it.
- **Adds a protocol without touching the core.** Each adapter owns its own framing, handshake, and schema logic. The routing core doesn't change to gain one.
- **Converts and checks UCI traffic.** Compile a UCI XSD catalog and the gateway converts OMS JSON ↔ UCI XML at the edge and checks payloads against the schema, from `warn` to `reject`.

## Getting started

Clone the repository and build it.

```bash
git clone https://github.com/OpenAutonomy/oa-gateway.git
cd oa-gateway
cargo build --release
```

Start the process with a configuration file. The host starts an adapter for each named table.

```bash
./target/release/oa-gateway config/default.toml
```

There is no authentication or authorization. TLS is available on the OWP listener and off by default. Bind loopback, as `config/default.toml` does, or put a reverse proxy in front. See [SECURITY.md](SECURITY.md).

In another terminal, send one message through the engine:

```bash
cargo run -p oa-gateway-bench --release -- ping --url ws://127.0.0.1:9000/
```

That does INIT (`002.5.0`), SUB `Ping` on `demo`, PUB `{"Ping":{"n":1}}`, waits for MSG, and exits 0.

To run the gateway in Docker, pass a configuration path:

```bash
OAG_CONFIG=$PWD/config/compose.toml docker compose -f compose/gateway.yml up --build
```

That serves OWP at `ws://127.0.0.1:9000/`. A local ActiveMQ broker is a separate stack (`compose/activemq.yml`). See [docs/connecting-active-mq.md](docs/connecting-active-mq.md).

## Standards

UCI, OMS, and A-GRA XSD documents are not in the tree. [`scripts/fetch-uci-schema.sh`](scripts/fetch-uci-schema.sh) writes a gitignored `schema/`. Tests use a fixture schema. The [Dockerfile](Dockerfile) fetches UCI 2.5 at image build. See [docs/using-custom-xsd.md](docs/using-custom-xsd.md).

## Config

A config is a TOML file passed as the only argument: one section per adapter, plus `[uci]` for the schema and `[engine]` for host reporting. Naming an adapter table turns it on; omitting it leaves it off. Settings you leave out fall back to built-in defaults; settings the gateway does not recognize are rejected at startup. The full key list, defaults, and shipped examples are in [docs/configuration.md](docs/configuration.md).

## Documentation

The guide and the crate API reference are both published at [openautonomy.github.io/oa-gateway](https://openautonomy.github.io/oa-gateway/).

- [Glossary](https://openautonomy.github.io/oa-gateway/guide/glossary.html) — acronyms and OA-Gateway's own vocabulary
- [Architecture](https://openautonomy.github.io/oa-gateway/guide/architecture.html) — crates, routing, and an example path
- [Configuration](https://openautonomy.github.io/oa-gateway/guide/configuration.html) — TOML sections, keys, and defaults
- [Writing an Adapter](https://openautonomy.github.io/oa-gateway/guide/writing-an-adapter.html) — adding a protocol
- [Using a Custom XSD](https://openautonomy.github.io/oa-gateway/guide/using-custom-xsd.html) — running against a custom message set
- [Connecting ActiveMQ](https://openautonomy.github.io/oa-gateway/guide/connecting-active-mq.html) — bridging an ActiveMQ broker
- [Connecting DDS](https://openautonomy.github.io/oa-gateway/guide/connecting-dds.html) — joining a DDS domain
- [Benchmarking](https://openautonomy.github.io/oa-gateway/guide/benchmarking.html) — latency and throughput utility, plus the CI `bench` artifact
- [rustdoc](https://openautonomy.github.io/oa-gateway/) — crate APIs, including the host binary's modules. Build locally with `cargo doc --workspace --no-deps --document-private-items --open`. `oa-gateway-adapter` includes a runnable minimal adapter.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Changelog

Notable changes are in [CHANGELOG.md](CHANGELOG.md).

## License

Licensed under the [MIT License](LICENSE), copyright John Henry Burns.

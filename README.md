# OA-Gateway

[![CI](https://github.com/OpenAutonomy/oa-gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/OpenAutonomy/oa-gateway/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

![OA-Gateway: multi-protocol gateway bridging ActiveMQ, DDS, and WebSockets](docs/oa-gateway-banner.png)

## Introduction

OA-Gateway is a multi-protocol message gateway written in Rust. It bridges WebSocket, ActiveMQ (STOMP), and DDS traffic through one protocol-agnostic routing core, and speaks OMS (Open Mission Systems), UCI (Universal Command and Control Interface), and A-GRA so it can interoperate with systems built on those standards.

It is an independent prototype released under the MIT License, and is not an [Open Arsenal](https://gitlab.com/open-arsenal/) project. Read [SECURITY.md](SECURITY.md) before exposing it to sensitive data.

## What it does

- **Bridges three protocols through one router.** WebSocket (OWP), STOMP (ActiveMQ), and DDS (rustdds) adapters all publish to and subscribe from the same in-process engine. A message published on one reaches every matching subscriber on every other.
- **Routes by name, not by peer.** Traffic is addressed by topic and message type the same way across every protocol, so adding a subscriber never means telling a publisher about it.
- **Adds a protocol without touching the core.** Each adapter owns its own framing, handshake, and schema logic. The routing core doesn't change to gain one.
- **Converts and checks UCI traffic.** Compile a UCI XSD catalog and the gateway converts OMS JSON ↔ UCI XML at the edge and checks payloads against the schema, from `warn` to `reject`.
- **Optional TLS at the network edges.** OWP and STOMP can terminate or originate TLS, and OWP can require a client certificate from a trusted CA. All of it is off by default.

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

`config/default.toml` binds to loopback and has no authorization or authentication. Read [SECURITY.md](SECURITY.md) before exposing it.

In another terminal, send one message through the running engine:

```bash
cargo run -p oa-gateway-bench --release -- ping --url ws://127.0.0.1:9000/
```

Docker, ActiveMQ, DDS, and the full configuration reference are in the [guide](https://openautonomy.github.io/oa-gateway/).

## Standards

UCI, OMS, and A-GRA XSD documents are not in the tree. [`scripts/fetch-uci-schema.sh`](scripts/fetch-uci-schema.sh) writes a gitignored `schema/`. Tests use a fixture schema. The [Dockerfile](Dockerfile) fetches UCI 2.5 at image build. See [docs/using-custom-xsd.md](docs/using-custom-xsd.md).

## Documentation

The user guide and the API reference are both published at [openautonomy.github.io/oa-gateway](https://openautonomy.github.io/oa-gateway/).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Changelog

Notable changes are in [CHANGELOG.md](CHANGELOG.md).

## License

Licensed under the [MIT License](LICENSE), copyright John Henry Burns.

# OA-Gateway

- [Introduction](#introduction)
- [Getting started](#getting-started)
- [Config](#config)
- [Documentation](#documentation)
- [FAQs](#faqs)
- [Contributing](#contributing)
- [License](#license)

## Introduction

OA-Gateway is an independent prototype: a multi-protocol connector implemented in Rust. It is open source and free to use under the MIT License. It is not an [Open Arsenal](https://gitlab.com/open-arsenal/) project; it speaks OMS, UCI, and A-GRA so it can sit next to those systems.

It currently supports WebSocket (OWP) and STOMP (ActiveMQ) adapters. The routing core is protocol-agnostic. Adapters own framing, handshake, and any schema logic. Supporting a new protocol means adding an adapter, not changing the core.

Goals:

- Compliance with the OMS, UCI, and A-GRA standards.
- Topic- and destination-level routing, so traffic is addressed by name rather than by peer.
- Ease of extensibility (i.e., adding new protocol adapters).

## Getting started

To get started, download and build the project.

```bash
git clone https://github.com/jburns3141/oa-gateway.git
cd oa-gateway
cargo build --release
```

Next, launch the service with your configuration file. This will create adapters for all of the configured connections.

```bash
./target/release/oa-gateway config/default.toml
```

To run the gateway in Docker instead (config path required):

```bash
OAG_CONFIG=$PWD/config/compose.toml docker compose -f compose/gateway.yml up --build
```

That serves OWP at `ws://127.0.0.1:9000/`. A local ActiveMQ broker is separate — `compose/activemq.yml` — see [docs/connecting-active-mq.md](docs/connecting-active-mq.md).

## Config

A config is a TOML file passed as the only argument: one section per adapter, plus `[uci]` for the schema and `[engine]` for host reporting. Naming an adapter table turns it on; omitting it leaves it off. Settings you leave out fall back to built-in defaults; settings the gateway does not recognize are rejected at startup. The full key list, defaults, and shipped examples are in [docs/configuration.md](docs/configuration.md).

## Documentation

- [docs/glossary.md](docs/glossary.md) — the acronyms, and OA-Gateway's own vocabulary
- [docs/configuration.md](docs/configuration.md) — TOML sections, keys, and defaults
- [docs/writing-an-adapter.md](docs/writing-an-adapter.md) — adding a protocol
- [docs/using-custom-xsd.md](docs/using-custom-xsd.md) — running against your own message set
- [docs/connecting-active-mq.md](docs/connecting-active-mq.md) — bridging an ActiveMQ broker
- rustdoc — crate APIs, including the host binary's modules. Locally: `cargo doc --workspace --no-deps --document-private-items --open`. CI builds the same tree on every run (download the `rustdoc` artifact) and publishes it from the default branch (GitHub Pages / GitLab Pages). `oa-gateway-adapter` carries a runnable minimal adapter.

## FAQs

1. [STOMP, UCI, ASB? What do all these terms mean?](docs/glossary.md)
1. [How do I configure the gateway?](docs/configuration.md)
1. [How can I add a new protocol?](docs/writing-an-adapter.md)
1. [How can I use my own XSD?](docs/using-custom-xsd.md)
1. [How can I connect my ActiveMQ instance?](docs/connecting-active-mq.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under the [MIT License](LICENSE) (copyright John Henry Burns). Open source and free to use.

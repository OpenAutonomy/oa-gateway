# OA-Gateway

- [Introduction](#introduction)
- [Getting started](#getting-started)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## Introduction

OA-Gateway is a multi-protocol connector for software systems in the [Open Arsenal](https://gitlab.com/open-arsenal/) ecosystem. The project is implemented in Rust and currently supports WebSocket and STOMP (ActiveMQ) protocol adapters. The routing core is protocol-agnostic. Adapters own framing, handshake, and any schema logic. Supporting a new protocol means simply adding an adapter, not changing the core.

Goals:

- Compliance with the OMS, UCI, and A-GRA standards.
- Topic- and destination-level routing, so traffic is addressed by name rather than by peer.
- Ease of extensibility (i.e., adding new protocol adapters).

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

To get started, download and build the project. The toolchain is pinned in `rust-toolchain.toml`, so `cargo` picks up the right compiler and there is nothing else to install.

```bash
git clone https://github.com/jburns3141/oa-gateway.git
cd oa-gateway
cargo build --release
```

Next, launch the service with your configuration file. This will create adapters for all of the configured connections.

```bash
./target/release/oa-gateway config/default.toml
```

A config is a short set of TOML sections: one per adapter, plus one naming the UCI schema. Each adapter section carries an `enabled` flag and an id, then whatever that protocol needs — an address to listen on or a broker to dial, which topics to bridge, and whether payloads are converted on the way through or passed along untouched. Settings you leave out fall back to built-in defaults, and settings the gateway does not recognize are rejected at startup rather than ignored.

Where a schema is named, payloads are also checked against it, and the same section decides what a departure from the standard costs: reported and carried anyway, refused and explained to the peer, or not checked at all. Converting is not the same as complying — a message can convert cleanly in both directions and still be missing elements the standard requires — so the check is worth having even where conversion already succeeds.

The files in `config/` are worked examples. The gateway has no TLS or authentication of its own, so bind to loopback — see [SECURITY.md](SECURITY.md) for what that means and what it assumes about the network around it.

## Documentation

- [docs/glossary.md](docs/glossary.md) — the acronyms, and OA-Gateway's own vocabulary
- [docs/writing-an-adapter.md](docs/writing-an-adapter.md) — adding a protocol
- [SECURITY.md](SECURITY.md) — deployment assumptions, reporting, known limitations
- `cargo doc --workspace --no-deps --open` — the crate APIs. `oa-gateway-adapter` carries a runnable minimal adapter.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under the [MIT License](LICENSE) (copyright John Henry Burns). Compatible with Open Arsenal A-GRA / sleet (Apache-2.0) and MIT projects such as Ghost Detector.

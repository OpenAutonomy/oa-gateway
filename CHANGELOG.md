# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-08-29

### Added

- `owp.init_timeout_secs` (default `30`) and `owp.idle_timeout_secs` (default
  `600`): the OWP listener closes a connection that never completes INIT, and
  an active session that goes silent in both directions, so a peer cannot hold
  a connection slot without making progress. The INIT deadline is measured from
  the handshake; the idle deadline resets on any client or server frame. `0`
  disables either timeout, which is the pre-existing behavior.
- `owp.allowed_origins`: an opt-in allowlist of exact `Origin` header values
  for the WebSocket handshake. Empty (the default) accepts any origin, as
  before; a non-empty list refuses a handshake whose `Origin` is not listed
  verbatim — a missing `Origin` included — with `403`, closing the
  cross-site-WebSocket path for browser clients.
- `fuzz/`: `cargo-fuzz` harnesses for the STOMP and OWP codecs, the A-GRA
  wrapper, and UCI JSON/XML conversion and XSD compilation, run on every
  change and weekly on a longer schedule.
- Tag-triggered release workflow: a `vX.Y.Z` push now builds a checksummed
  binary, a CycloneDX SBOM, a GitHub Release, and a `ghcr.io` image. See
  [RELEASING.md](RELEASING.md).

### Changed

- The STOMP client's `passcode` is held as a redacting `Secret` type, so it
  can no longer appear in a `Debug` of the adapter or host config.
- `[workspace.lints]` now forbids `unsafe` and enforces a curated
  `clippy::pedantic` set across every crate.

## [0.2.0] - 2026-08-28

### Added

- The `docs/*.md` guide is now published to GitHub Pages alongside rustdoc,
  built with mdBook (`book.toml`, `docs/SUMMARY.md`) under `/guide/`. The
  CI artifact that carries both is renamed from `rustdoc` to `site`.
- `owp.reconnect`, `owp.reconnect_delay_secs`, `owp.on_panic`: the OWP
  adapter can rebind and keep accepting after its accept loop ends or
  panics, matching STOMP. Defaults preserve today's no-retry behavior.
- `dds.reconnect`, `dds.reconnect_delay_secs`, `dds.on_panic`: the same,
  for rejoining the domain after the DDS adapter's session ends or panics.
- `dds.max_sample_size`: an oversized inbound DDS sample is dropped and
  logged instead of being unwrapped or converted.
- DDS now checks inbound samples against `[uci].schema` and `[uci].validate`
  the same way OWP traffic is checked. `reject` drops a non-conforming
  sample and logs it; DDS has no peer connection to answer with an error.
- `owp.tls_cert`, `owp.tls_key`: the OWP listener terminates TLS when both
  are set, so a client connects with `wss://` instead of needing a reverse
  proxy in front. Encryption and server identity only — clients are still
  not authenticated. Unset by default, and an unset pair leaves the
  listener byte-for-byte as it was.
- `stomp.tls`, `stomp.tls_ca`, `stomp.tls_server_name`: the STOMP client can
  dial a broker over TLS, so `login` and `passcode` no longer have to cross
  in the clear. The broker's certificate is verified against `tls_ca`, or
  the operating system trust store when that is empty. Off by default.
- `owp.tls_client_ca`: the OWP listener can require and verify a client
  certificate from that CA, refusing anything else at the handshake —
  mutual TLS, and the one form of peer authentication this gateway has.
  Off by default, requires `tls_cert`/`tls_key`, and stops at the
  handshake: an authenticated client is not treated any differently from
  before, since there is still no authorization.
- `stomp.tls_client_cert`, `stomp.tls_client_key`: the STOMP client can
  present a certificate to the broker, for a broker whose SSL transport
  connector requires one (ActiveMQ's `needClientAuth`). This authenticates
  the gateway to the broker, the same role `login`/`passcode` already play.
  Off by default, requires `tls = true`.

### Fixed

- OWP now validates the JSON it delivers to an `xml_baseline` client after
  converting from bus XML, not only the pre-conversion XML. `uci.validate`
  governs a conversion-produced violation — for example an A-GRA wrapper's
  `EncodedPayload` changing length once its inner document is re-encoded from
  XML to JSON — the same way it governs a producer-caused one.
- A-GRA wrapper fields (`MessageType`, `EncodedPayload`,
  `DestinationRouting`, nested `UUID`s) are now located by parsing the
  wrapper XML with `roxmltree` instead of substring search, closing a
  class of misdirected-field-extraction bug on adversarial input.
- Workspace rustdoc landing page uses rustdoc's CSS so the Pages root
  matches the crate docs.

## [0.1.0] - 2026-08-17

First public release. Distributed as a git tag and a GitHub Release, not on
crates.io. Canonical home is
[OpenAutonomy/oa-gateway](https://github.com/OpenAutonomy/oa-gateway).

### Added

- OWP adapter: WebSocket sessions, INIT / SUB / PUB / MSG, JSON and XML payloads.
- STOMP adapter: ActiveMQ destinations and a frame codec.
- DDS adapter: rustdds provider and domain join.
- UCI convert and validate: JSON / XML conversion from a compiled schema;
  `uci.validate` can warn or reject.
- A-GRA unwrap: wrapper fields located and the inner payload handed to routing.
- Compose files for a local gateway host and a local ActiveMQ broker.
- `oa-gateway-bench` for engine, loopback, OWP, and UCI timing, plus `ping`
  against a running gateway.
- rustdoc on GitHub Pages:
  [openautonomy.github.io/oa-gateway](https://openautonomy.github.io/oa-gateway/).

[Unreleased]: https://github.com/OpenAutonomy/oa-gateway/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/OpenAutonomy/oa-gateway/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/OpenAutonomy/oa-gateway/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/OpenAutonomy/oa-gateway/releases/tag/v0.1.0

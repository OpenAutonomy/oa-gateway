# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `scripts/fetch-agra-schema.sh`, pinned by checksum to the public
  `open-arsenal/a-gra/standard` `v5.0a` tag (not `main`, which has since moved
  to A-GRA 6.0a). CI now runs `oa-gateway-uci`'s `tests/agra_schema.rs` against
  the real fetched schema instead of silently skipping it for lack of a
  sibling `open-ma` checkout.
- `oa-gateway-stomp` now validates against the compiled XSD too, in both
  directions: an inbound MESSAGE frame before it reaches the engine, and an
  outbound SEND frame before it reaches the broker. It was the one adapter
  that forwarded bytes unchecked; the outbound side matters in particular
  because `uci.validate` defaults to `warn`, not `reject`, so a
  non-conforming payload from any other adapter already crosses the
  engine today and would otherwise reach the real ActiveMQ bus unchecked.
  `StompConfig` gains `schema`/`xsd_validator`/`validate` fields, matching
  `DdsConfig`'s shape; a rejected frame is dropped and logged, the same
  way DDS already handles a rejected inbound sample, since STOMP has no
  peer connection to answer the way OWP's WebSocket session can.
- `oa-gateway-xsd-libxml`, a new crate that validates converted UCI/A-GRA XML
  against a real XSD using libxml2 (via the `libxml` crate), compiled once at
  startup on its own OS thread and called from `oa-gateway-owp` and
  `oa-gateway-dds`. This is the one place `unsafe` enters the dependency tree
  — libxml2's own FFI, not this workspace's code, which still forbids
  `unsafe` outright. `oa-gateway`, the Dockerfile's builder stage, and CI all
  now need `libxml2-dev`, `pkg-config`, and `libclang-dev` to build; the
  Dockerfile's runtime stage needs `libxml2`.

### Changed

- UCI/A-GRA schema validation moved from a hand-rolled, from-scratch
  validator to libxml2, a mature, independently-maintained XSD 1.0 engine.
  It is strictly more complete: it enforces the full facet and pattern
  language rather than a recognized subset, and checks identity constraints
  (`xs:unique`, `xs:key`) the hand-rolled validator never read at all.
  `uci.validate`'s `off`/`warn`/`reject` modes are unchanged and now gate the
  libxml2 result directly. Conversion (`oa-gateway-uci`'s `Schema`/
  `xsd::compile`/JSON ↔ XML) is untouched — libxml2 has no JSON capability of
  any kind, so this bespoke mapping stays exactly as it was.

### Removed

- `oa-gateway-uci`'s hand-rolled validator (`validate::validate`,
  `Violation`, `ViolationKind`, and `Message::violations()`) and its ~600
  lines of validation logic plus ~638 lines of tests, replaced by
  `oa-gateway-xsd-libxml` above. `oa-gateway-uci` keeps only `Mode`
  (`ValidateMode`) and the schema-aware codec; it has no validator of its
  own anymore and is purely a conversion library.
- The two startup warnings `values of these types will not be checked
  beyond the facets on them` and `some schema patterns cannot be checked and
  will not be enforced`. Both described gaps in the deleted hand-rolled
  validator specifically; libxml2 validates directly against the real XSD
  text and was never subject to either gap, so the warnings no longer
  corresponded to anything true about this build's actual validation
  behavior.

### Fixed

- `oa-gateway-uci`'s XML writer (`Message::to_xml`) emitted a complex
  type's child elements in whatever order they happened to arrive in —
  alphabetical for JSON-origin messages, since `serde_json`'s
  `Value::Object` is `BTreeMap`-backed — instead of the schema's declared
  `xs:sequence` order. The hand-rolled validator never caught this (it
  checks presence and cardinality, not order), so a message could convert,
  report zero violations, and still be invalid XML per a real XSD
  validator. Found via libxml2 correctly rejecting all four golden A-GRA
  fixtures on this exact ground. Elements are now written in schema-declared
  order; an undeclared field (which validation, not conversion, is where
  that is named) is still written, just after every declared one.
- `scripts/fetch-uci-schema.sh` pointed at `main` on
  `open-arsenal/uci/standard`, which has since moved to UCI 2.6 with a
  reorganized tree — the `OAC-STD-UCI_V2.5/` directory it fetched from no
  longer exists there, so every fresh fetch (a clean `docker build` included)
  404'd. Re-pinned to the `v2.5` tag, where that directory still lives
  byte-identical to what `scripts/uci-schema.sha256` already pinned.
- A broken intra-doc link in `oa-gateway-uci`'s `Mode::is_on` doc comment,
  left over from deleting `validate::validate` — `cargo doc -D warnings`
  failed on it in CI.
- `rustls` (0.23.43 → 0.23.46), `rustls-webpki` (0.103.15 → 0.103.17), and
  `tokio` (1.53.1 → 1.54.1), each one patch release behind a RustSec
  advisory published since this repo's last successful `cargo-deny` run
  (RUSTSEC-2026-0285, -0346, -0348, -0349, -0382, -0386) — pre-existing and
  unrelated to this change, caught only because this PR's CI run was the
  first to check advisories since they were published. All six are fixed by
  an in-range `cargo update`, in both `Cargo.lock` and `fuzz/Cargo.lock`.

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

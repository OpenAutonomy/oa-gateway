# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

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

[Unreleased]: https://github.com/OpenAutonomy/oa-gateway/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OpenAutonomy/oa-gateway/releases/tag/v0.1.0

# Contributing

MIT prototype. PRs / MRs welcome.

Start with [docs/glossary.md](docs/glossary.md) for the acronyms and [docs/writing-an-adapter.md](docs/writing-an-adapter.md) if you are adding a protocol.

## Gate

Pinned toolchain is in `rust-toolchain.toml` (`1.85.0`, plus `rustfmt` and `clippy`).

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

That is what CI runs on every push and pull/merge request (GitHub Actions and GitLab CI).

The gate needs no UCI schema: conversion tests use a small fixture schema. To exercise the compiler against the real 722-message catalog, fetch the standard once and the ignored test finds it:

```bash
./scripts/fetch-uci-schema.sh
cargo test -p oa-gateway-uci -- --ignored
```

Optional, needs Docker:

```bash
./scripts/live-activemq.sh
```

CI also runs the ignored ActiveMQ XML round-trip after the unit gate.

## Notes

- Do not compile against or commit `repos/` — those clones are reference only. The schema the gateway loads comes from `scripts/fetch-uci-schema.sh`, which lands in a gitignored `schema/`.
- Cross-adapter tests and fixtures live in `crates/oa-gateway-testing`. Adapter crates keep unit tests only.
- New behavior needs a test that asserts observable routing or codec output, not source layout.
- Commit messages: why, not how.

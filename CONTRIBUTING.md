# Contributing

Independent prototype, MIT licensed. PRs / MRs welcome.

Start with [docs/glossary.md](docs/glossary.md) for the acronyms and [docs/writing-an-adapter.md](docs/writing-an-adapter.md) if you are adding a protocol.

Found a vulnerability? Do not open a PR for it first — [SECURITY.md](SECURITY.md)
says how to report it, and lists the resource limits already known to be missing
so you can tell a finding from a chore.

## Gate

Pinned toolchain is in `rust-toolchain.toml` (`1.85.0`, plus `rustfmt` and `clippy`).

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --document-private-items
./scripts/write-rustdoc-index.sh target/doc
```

That is what CI runs on every push and pull/merge request (GitHub Actions and GitLab CI). The rustdoc HTML is uploaded as a `rustdoc` artifact. On a public default branch, GitHub Pages also publishes it (set the source to GitHub Actions on first use). A private repository on a free plan cannot host Pages; download the artifact instead. GitLab Pages still publishes from the default branch.

Touching dependencies also runs `cargo deny check` against the policy in
`deny.toml` — advisories, licenses, and sources. Install it with
`cargo install --locked cargo-deny` (or `brew install cargo-deny`) to see a
failure before pushing. Adding a dependency under a license the policy does not
list is meant to fail: say in the PR why the license is acceptable, and add it.

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

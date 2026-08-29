# Contributing

OA-Gateway is an independent prototype under the MIT License. Pull requests are welcome.

By participating in this project you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

Start with [docs/glossary.md](docs/glossary.md) for the acronyms, [docs/architecture.md](docs/architecture.md) for the crate graph, and [docs/writing-an-adapter.md](docs/writing-an-adapter.md) if you are adding a protocol.

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
mdbook-mermaid install .
mdbook build
```

`mdbook build` needs [mdBook](https://github.com/rust-lang/mdBook) and [mdbook-mermaid](https://github.com/badboy/mdbook-mermaid) on `PATH` (`cargo install mdbook mdbook-mermaid --locked`, or `brew install mdbook` plus a downloaded `mdbook-mermaid` release — it isn't in Homebrew). `mdbook-mermaid install .` vendors `mermaid.min.js`/`mermaid-init.js` at the repo root (gitignored, regenerated on demand) so `architecture.md`'s diagrams render; `mdbook build` then renders `docs/*.md` (configured in `book.toml`) into `target/doc/guide`.

That is what CI runs on every push and pull request. The rustdoc HTML and the built guide are uploaded together as a `site` artifact. On a public default branch, GitHub Pages also publishes it (set the source to GitHub Actions on first use). A private repository on a free plan cannot host Pages; download the artifact instead.

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

The following script is optional and needs Docker:

```bash
./scripts/live-activemq.sh
```

CI also runs the ignored ActiveMQ XML round-trip after the unit gate.

After the unit job, CI runs [`scripts/ci-bench.sh`](scripts/ci-bench.sh) and uploads a `bench` artifact (JSON, `summary.txt`, and PNGs when gnuplot is present). That job is a snapshot, not a performance gate. Local long runs and how to read drops are in [docs/benchmarking.md](docs/benchmarking.md).

A `coverage` job runs `cargo llvm-cov` and uploads an lcov + HTML report as a `coverage` artifact. It has no threshold and does not gate anything — it is there to see line coverage per crate. Run it locally with `cargo install cargo-llvm-cov --locked` then `cargo llvm-cov --workspace --html --open`.

## Notes

- Do not compile against or commit `repos/` — those clones are reference only. The schema the gateway loads comes from `scripts/fetch-uci-schema.sh`, which lands in a gitignored `schema/`.
- Cross-adapter tests and fixtures live in `crates/oa-gateway-testing`. Adapter crates keep unit tests only.
- New behavior needs a test that asserts observable routing or codec output, not source layout.
- Commit messages: why, not how.
- User-facing changes belong under `[Unreleased]` in [CHANGELOG.md](CHANGELOG.md).

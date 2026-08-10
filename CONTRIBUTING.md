# Contributing

Apache-2.0 prototype. PRs / MRs welcome.

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

Optional, needs Docker:

```bash
./scripts/live-activemq.sh
```

CI also runs the ignored ActiveMQ XML round-trip after the unit gate.

## Notes

- Do not compile against or commit `repos/` — those clones are reference only.
- Cross-adapter tests and fixtures live in `crates/mpg-testing`. Adapter crates keep unit tests only.
- New behavior needs a test that asserts observable routing or codec output, not source layout.
- Commit messages: why, not how.

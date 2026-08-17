## Summary

<!-- Why this change, not how. -->

## Gate

From [CONTRIBUTING.md](https://github.com/jburns3141/oa-gateway/blob/main/CONTRIBUTING.md):

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] rustdoc (`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --document-private-items`)

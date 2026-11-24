# Fuzzing

`cargo-fuzz` harnesses for the peer-facing parsers — the code that turns bytes
off a socket into a message. Every finding in `SECURITY.md`'s "In scope" list is
one of these.

| target | exercises |
| --- | --- |
| `stomp_codec` | `oa_gateway_stomp::decode_one_with_limit` — the STOMP frame decoder |
| `owp_codec` | `oa_gateway_owp::parse_client` — OWP `INIT` / `PUB` / `SUB` / `UNSUB` |
| `agra_wrapper` | `oa_gateway_agra::{wrapper_kind, unwrap, xml_root_local_name}` |

This is a separate cargo workspace: it needs a nightly toolchain and the
`libfuzzer-sys` runtime, which the main workspace does not want.

```sh
rustup toolchain install nightly
cargo +nightly install cargo-fuzz --locked

./fuzz/seed-corpus.sh                       # fixtures -> fuzz/corpus/<target>/
cargo +nightly fuzz run agra_wrapper        # runs until a crash or Ctrl-C
cargo +nightly fuzz run agra_wrapper -- -max_total_time=60   # time-boxed
```

A crash writes the offending input to `fuzz/artifacts/<target>/`; replay it with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`. `corpus/`,
`artifacts/`, and `target/` are gitignored.

CI runs each target for 60 s on every change and 5 min on a weekly schedule
(`.github/workflows/fuzz.yml`).

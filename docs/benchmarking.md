# Benchmarking

`oa-gateway-bench` measures publish-to-delivery latency, throughput, and engine drop counts. It is a client of the public APIs. It does not change routing, framing, or conversion.

Shared GitHub runners are noisy. Numbers from CI are a snapshot, not a regression gate. Compare two runs only when the machine and the flags match.

## Scenarios

Each command isolates one cost layer. In-process runs share a clock, so one-way latency is `Instant` at send minus `Instant` at receive. Sequence numbers live in the JSON payload (`n`), not in engine headers.

| Command | Path | What the latency is |
|---|---|---|
| `engine` | `Engine::publish` → subscriber channels | One-way to `recv` |
| `loopback` | Loopback A → engine → Loopback B | One-way, including the extra forwarder task |
| `owp` | WebSocket `PUB` → `MSG` | One-way; `--ack-latency` also times `PUB` → `+OK` |
| `uci` | `Message::from_json` / `to_xml` (or the reverse) on the PositionReport fixture | Convert time |

`engine --capacity` is the channel the bench passes to the existing `Engine::subscribe`. The default is 4096 so the run measures routing, not the production 64-slot `try_send` drop policy. `engine --capacity 64` is how you measure backpressure. Loopback keeps its hardcoded 64-slot channels; there is no new parameter on that adapter.

`owp --xml-baseline` starts the same in-process OWP helper the tests use (`oa_gateway_uci::slice::v25`) and sends PositionReport so conversion is on the path. `owp --url ws://127.0.0.1:9000/` attaches two connections to a running host instead. Handshake schema is `002.5.0`.

STOMP and DDS are not in this utility. They need a broker or a domain; use the live ActiveMQ script when you want that path.

## Commands

```bash
cargo run -p oa-gateway-bench --release -- engine --duration 10s --warmup 1s
cargo run -p oa-gateway-bench --release -- engine --capacity 64
cargo run -p oa-gateway-bench --release -- loopback --duration 10s
cargo run -p oa-gateway-bench --release -- owp --duration 10s
cargo run -p oa-gateway-bench --release -- owp --xml-baseline --duration 10s
cargo run -p oa-gateway-bench --release -- owp --url ws://127.0.0.1:9000/
cargo run -p oa-gateway-bench --release -- uci --iterations 2000
```

`--rate 0` (the default) means as-fast-as-possible. `--json PATH` writes the same numbers the summary prints, plus `git_sha`, `rustc`, `profile`, `started_unix`, and the flags.

`--warmup` is a prefix of `--duration`. Sends in that window still count toward `sent` / `received`; they are omitted from the latency histogram.

A run exits non-zero if the binary cannot start, a handshake fails, or nothing is received. It does not exit non-zero because a percentile moved.

## Reading drops

`Engine::publish` uses `try_send`. A full subscriber channel increments `dropped` and the message is gone. The summary prints both the per-run drop count and, for embedded scenarios, `EngineStats` (`published`, `delivered`, `dropped`). High throughput with `--capacity 64` or the loopback adapter will drop. That is the backpressure policy, not a bench bug.

## CI

The unit job does not run the long suite. After `test` passes, a `bench` job runs [`scripts/ci-bench.sh`](../scripts/ci-bench.sh): a release build, five short scenarios (5s + 1s warmup, or 2000 UCI iterations), JSON under `bench/`, and `bench/summary.txt`.

If `jq` and `gnuplot` are available (the script installs them when it can), it also writes `bench/latency.png` and `bench/throughput.png`. A missing plotter does not fail the job.

GitHub uploads the `bench/` directory as a `bench` artifact (14 days). Pull requests upload too, so a reviewer can download the PR zip next to one from the default branch. Do not treat those pictures as a pass/fail signal.

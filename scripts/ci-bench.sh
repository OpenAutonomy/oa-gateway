#!/usr/bin/env bash
# Release bench suite for CI. Writes bench/*.json, bench/summary.txt, and
# optional PNGs. Fails only if a scenario exits non-zero.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${OUT:-bench}"
mkdir -p "$OUT"

cargo build -p oa-gateway-bench --release --locked
BIN=./target/release/oa-gateway-bench

: >"$OUT/summary.txt"
run() {
  local name="$1"
  shift
  echo "=== $name ===" | tee -a "$OUT/summary.txt"
  "$BIN" "$@" --json "$OUT/${name}.json" | tee -a "$OUT/summary.txt"
  echo | tee -a "$OUT/summary.txt"
}

run engine engine --duration 5s --warmup 1s
run loopback loopback --duration 5s --warmup 1s
run owp owp --duration 5s --warmup 1s
run owp-xml-baseline owp --duration 5s --warmup 1s --xml-baseline
run uci uci --iterations 2000

install_plot_tools() {
  if command -v jq >/dev/null 2>&1 && command -v gnuplot >/dev/null 2>&1; then
    return 0
  fi
  if command -v sudo >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y -qq jq gnuplot || return 1
  elif command -v apt-get >/dev/null 2>&1; then
    apt-get update -qq && apt-get install -y -qq jq gnuplot || return 1
  else
    return 1
  fi
}

plot_charts() {
  command -v jq >/dev/null 2>&1 || return 1
  command -v gnuplot >/dev/null 2>&1 || return 1

  {
    echo -e "scenario\tp50\tp90\tp99"
    for f in "$OUT"/engine.json "$OUT"/loopback.json "$OUT"/owp.json "$OUT"/owp-xml-baseline.json "$OUT"/uci.json; do
      [ -f "$f" ] || continue
      jq -r '[.scenario, ((.latency_ns.p50 // 0)/1000), ((.latency_ns.p90 // 0)/1000), ((.latency_ns.p99 // 0)/1000)] | @tsv' "$f"
    done
  } >"$OUT/latency.tsv"

  {
    echo -e "scenario\trecv_per_sec"
    for f in "$OUT"/engine.json "$OUT"/loopback.json "$OUT"/owp.json "$OUT"/owp-xml-baseline.json "$OUT"/uci.json; do
      [ -f "$f" ] || continue
      jq -r '[.scenario, .received_per_sec] | @tsv' "$f"
    done
  } >"$OUT/throughput.tsv"

  gnuplot <<EOF
set terminal pngcairo size 1000,600
set output '$OUT/latency.png'
set style data histogram
set style histogram clustered
set style fill solid border -1
set boxwidth 0.8
set ylabel 'latency (µs)'
set xlabel 'scenario'
set key top left
set xtics rotate by -15
plot '$OUT/latency.tsv' using 2:xtic(1) title 'p50', \
     '' using 3 title 'p90', \
     '' using 4 title 'p99'

set output '$OUT/throughput.png'
set ylabel 'received / s'
set key off
plot '$OUT/throughput.tsv' using 2:xtic(1) title 'recv/s'
EOF
}

if install_plot_tools && plot_charts; then
  echo "wrote $OUT/latency.png and $OUT/throughput.png"
else
  echo "skipping charts (jq/gnuplot unavailable)" >&2
fi

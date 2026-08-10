#!/usr/bin/env bash
# Bring up Classic, wait for STOMP, run the ignored live XML round-trip.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

COMPOSE=(docker compose -f compose/activemq.yml)
"${COMPOSE[@]}" up -d

echo "waiting for STOMP 127.0.0.1:61613 …"
for _ in $(seq 1 40); do
  if (echo >/dev/tcp/127.0.0.1/61613) >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done
if ! (echo >/dev/tcp/127.0.0.1/61613) >/dev/null 2>&1; then
  echo "ActiveMQ STOMP port never opened" >&2
  "${COMPOSE[@]}" logs --tail 80
  exit 1
fi
# Broker accepts TCP before STOMP CONNECTED is reliable.
sleep 1

export MPG_ACTIVEMQ_STOMP="${MPG_ACTIVEMQ_STOMP:-127.0.0.1:61613}"
cargo test -p mpg-testing --test live_activemq -- --ignored --nocapture --test-threads=1

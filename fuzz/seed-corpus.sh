#!/usr/bin/env bash
# Populate fuzz/corpus/<target>/ from the repo's test fixtures and a few
# hand-written valid frames. Idempotent; safe to run before every fuzz run.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

seed() {
    local target="$1"
    shift
    mkdir -p "$here/corpus/$target"
    for src in "$@"; do
        [ -e "$src" ] && cp -f "$src" "$here/corpus/$target/" || true
    done
}

# UCI instance + XSD: the JSON/XML fixtures the unit tests already use.
seed uci_instance \
    "$root"/crates/oa-gateway-testing/fixtures/PositionReport.json \
    "$root"/crates/oa-gateway-testing/fixtures/PositionReport.xml \
    "$root"/crates/oa-gateway-uci/tests/fixtures/*.json
seed uci_xsd "$root"/crates/oa-gateway-uci/tests/fixtures/*.json

# A-GRA: the RxDataPayload fixture is a real wrapper document.
seed agra_wrapper "$root"/crates/oa-gateway-uci/tests/fixtures/MA_RxDataPayload.json

# Codec targets: no fixture files, so drop in one valid frame each.
mkdir -p "$here/corpus/stomp_codec" "$here/corpus/owp_codec"
printf 'SEND\ndestination:/topic/demo\ncontent-length:15\n\n{"Ping":{"n":1}}\0' \
    > "$here/corpus/stomp_codec/send.frame"
printf 'INIT {"versions":["1.0"],"schema":"002.5.0","service_id":"web"}' \
    > "$here/corpus/owp_codec/init"
printf 'PUB demo {"Ping":{"n":1}}' > "$here/corpus/owp_codec/pub"

echo "corpus seeded under $here/corpus/"

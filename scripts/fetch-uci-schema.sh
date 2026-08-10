#!/usr/bin/env bash
# Fetch the UCI 2.5 schema documents that oa-gateway converts against.
#
# The standard is public — Distribution Statement A, approved for public release —
# but it is not redistributed in this repository. Files land in schema/uci/, which
# is gitignored, and are checked against the digests pinned in
# scripts/uci-schema.sha256 so you always know which revision you are running.
#
# Override the source with OAG_UCI_SCHEMA_URL or the destination with
# OAG_UCI_SCHEMA_DIR, for a program-specific Message Set or an offline mirror.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BASE="${OAG_UCI_SCHEMA_URL:-https://gitlab.com/open-arsenal/uci/standard/-/raw/main/OAC-STD-UCI_V2.5}"
DEST="${OAG_UCI_SCHEMA_DIR:-schema/uci}"
DIGESTS="$ROOT/scripts/uci-schema.sha256"

# Both are required: the message definitions include the security markings, and
# compiling without them leaves types unresolved.
FILES=(UCI_MessageDefinitions_v2_5_0.xsd UCI_SecurityMarkings_v2_5_0.xsd)

if command -v sha256sum >/dev/null 2>&1; then
  CHECK=(sha256sum -c --status)
  GEN=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
  CHECK=(shasum -a 256 -c --status)
  GEN=(shasum -a 256)
else
  echo "need sha256sum or shasum to verify the download" >&2
  exit 1
fi

verify() {
  [ -d "$DEST" ] || return 1
  for f in "${FILES[@]}"; do
    [ -f "$DEST/$f" ] || return 1
  done
  (cd "$DEST" && "${CHECK[@]}" "$DIGESTS") >/dev/null 2>&1
}

if verify; then
  echo "schema already present and verified in $DEST/"
else
  mkdir -p "$DEST"
  for f in "${FILES[@]}"; do
    echo "fetching $f …"
    tmp="$(mktemp)"
    # Download to a temporary file so an interrupted run cannot leave a partial
    # schema behind that later looks like a checksum failure.
    if ! curl -sSfL --max-time 300 "$BASE/$f" -o "$tmp"; then
      rm -f "$tmp"
      echo "could not download $BASE/$f" >&2
      exit 1
    fi
    mv "$tmp" "$DEST/$f"
    chmod 644 "$DEST/$f"
  done

  if ! verify; then
    {
      echo
      echo "Checksum mismatch: $BASE"
      echo "does not serve what scripts/uci-schema.sha256 pins. Upstream has most"
      echo "likely published a new revision."
      echo
      echo "Review the change, then re-pin with:"
      echo "  (cd $DEST && ${GEN[*]} ${FILES[*]}) > scripts/uci-schema.sha256"
    } >&2
    exit 1
  fi
fi

cat <<EOF

Schema ready. config/asb.toml already points here; for another config use:

[uci]
schema = [
$(for f in "${FILES[@]}"; do echo "  \"$DEST/$f\","; done)
]
EOF

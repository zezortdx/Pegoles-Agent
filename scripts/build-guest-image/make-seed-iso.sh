#!/usr/bin/env bash
# Build the NoCloud seed ISO (volume label `cidata`) carrying cloud-init's
# user-data/meta-data plus the guest runtime binary. BUILD TIME ONLY: the
# normal lifecycle never attaches a seed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SEED_DIR="$ROOT/scripts/build-guest-image/seed"
RUNTIME_BIN="${1:?usage: make-seed-iso.sh <pegoles-guest-runtime-binary> <output.iso>}"
OUT_ISO="${2:?usage: make-seed-iso.sh <pegoles-guest-runtime-binary> <output.iso>}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp "$SEED_DIR/user-data" "$SEED_DIR/meta-data" "$WORK/"
cp "$RUNTIME_BIN" "$WORK/pegoles-guest-runtime"
chmod 644 "$WORK/user-data" "$WORK/meta-data" "$WORK/pegoles-guest-runtime"

rm -f "$OUT_ISO"
hdiutil makehybrid -iso -joliet -o "$OUT_ISO" -default-volume-name cidata "$WORK" >/dev/null
echo "seed ISO at $OUT_ISO ($(du -h "$OUT_ISO" | cut -f1))"

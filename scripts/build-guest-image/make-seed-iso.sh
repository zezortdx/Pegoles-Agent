#!/usr/bin/env bash
# Build the NoCloud seed ISO (volume label `cidata`) carrying cloud-init's
# user-data/meta-data plus the guest runtime + fixture binaries and the
# version-pinned .deb bundle. BUILD TIME ONLY: the normal lifecycle never
# attaches a seed.
#
# usage: make-seed-iso.sh <runtime-bin> <fixture-bin> <debs-dir> <output.iso>
#   (legacy 2-arg form still builds a v0.1-style seed: runtime only)
# Images 0.4 / x64-0.2 (browser): PEGOLES_FORWARDER_BIN=<pegoles-egress-forwarder>
# is required with the 4-arg form (seed/browser, seed/weston and the .path
# unit always ship).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SEED_DIR="$ROOT/scripts/build-guest-image/seed"
RUNTIME_BIN="${1:?usage: make-seed-iso.sh <runtime-bin> [fixture-bin] [debs-dir] <output.iso>}"

if [ "$#" -eq 2 ]; then
  OUT_ISO="$2"
  EXTRA=0
else
  FIXTURE_BIN="${2:?usage: make-seed-iso.sh <runtime-bin> [fixture-bin] [debs-dir] <output.iso>}"
  DEBS_DIR="${3:?usage: make-seed-iso.sh <runtime-bin> [fixture-bin] [debs-dir] <output.iso>}"
  OUT_ISO="${4:?usage: make-seed-iso.sh <runtime-bin> [fixture-bin] [debs-dir] <output.iso>}"
  EXTRA=1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp "$SEED_DIR/user-data" "$SEED_DIR/meta-data" "$WORK/"
cp "$RUNTIME_BIN" "$WORK/pegoles-guest-runtime"
mkdir -p "$WORK/units"
cp "$SEED_DIR"/units/*.service "$SEED_DIR"/units/*.path "$WORK/units/"
mkdir -p "$WORK/browser" "$WORK/weston"
cp "$SEED_DIR"/browser/* "$WORK/browser/"
cp "$SEED_DIR"/weston/weston.ini "$WORK/weston/"
mkdir -p "$WORK/sysctl"
cp "$SEED_DIR"/sysctl/*.conf "$WORK/sysctl/"
chmod 644 "$WORK/user-data" "$WORK/meta-data" "$WORK/pegoles-guest-runtime" "$WORK"/units/* "$WORK"/browser/* "$WORK/weston/weston.ini"

if [ "$EXTRA" -eq 1 ]; then
  FORWARDER_BIN="${PEGOLES_FORWARDER_BIN:?set PEGOLES_FORWARDER_BIN (pegoles-egress-forwarder, see build-runtime.sh)}"
  cp "$FORWARDER_BIN" "$WORK/pegoles-egress-forwarder"
  chmod 644 "$WORK/pegoles-egress-forwarder"
  cp "$FIXTURE_BIN" "$WORK/pegoles-input-fixture"
  chmod 644 "$WORK/pegoles-input-fixture"
  mkdir -p "$WORK/debs"
  cp "$DEBS_DIR"/*.deb "$WORK/debs/"
  cp "$DEBS_DIR/VERSIONS.txt" "$WORK/debs/VERSIONS.txt"
  echo "seed payload: runtime + fixture + $(ls "$WORK"/debs/*.deb | wc -l) debs"
fi

rm -f "$OUT_ISO"
if command -v hdiutil >/dev/null; then
  hdiutil makehybrid -iso -joliet -o "$OUT_ISO" -default-volume-name cidata "$WORK" >/dev/null
else
  xorriso -as mkisofs -quiet -V cidata -J -r -o "$OUT_ISO" "$WORK"
fi
echo "seed ISO at $OUT_ISO ($(du -h "$OUT_ISO" | cut -f1))"

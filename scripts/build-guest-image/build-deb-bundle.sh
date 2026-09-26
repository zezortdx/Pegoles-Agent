#!/usr/bin/env bash
# Debian package bundle for Pegoles Guest Image v2.
# BUILD TOOLING ONLY (Docker + official apt). The runtime stays offline:
# Debian's apt verifies signed InRelease metadata inside the disposable
# container; only the resulting verified .debs land on the seed ISO.
#
# Usage: bash scripts/build-guest-image/build-deb-bundle.sh [OUT_DIR]
#   (default OUT_DIR: a fresh private temp dir)
# Output: $OUT_DIR/*.deb + VERSIONS.txt (name/version/arch of every .deb).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/build-guest-image/common.sh
. "$ROOT/scripts/build-guest-image/common.sh"
OUT="$(out_dir "${1:-}")"

# Minimal graphical stack (trixie arm64, 2026-09). Chromium is NOT
# included (Phase 5.1 needs no browser). The top-level packages are pinned
# to exact versions: when the archive has moved on, apt fails instead of
# silently taking a newer build; bump deliberately with a manifest note.
# Their dependency closure is whatever the signed archive serves at build
# time and is recorded, per .deb, in VERSIONS.txt (sealed with the image).
TOP_PKGS="weston=14.0.2-1 foot=1.21.0-2 fonts-dejavu-core=2.37-8 xkb-data=2.42-1"

docker run --rm --platform linux/arm64 -v "$OUT:/out" "$DEBIAN_IMAGE" bash -c "
  set -euo pipefail
  apt-get update -q
  rm -f /out/*.deb
  apt-get install --download-only -y --no-install-recommends \
    -o Dir::Cache::Archives=/out $TOP_PKGS
  rm -f /out/lock /out/partial/*
  for f in /out/*.deb; do
    dpkg-deb -f \"\$f\" Package Version Architecture | tr '\n' ' '; echo
  done | sort > /out/VERSIONS.txt
"
echo "bundle at $OUT: $(find "$OUT" -maxdepth 1 -name '*.deb' | wc -l | tr -d ' ') debs, $(du -sh "$OUT" | cut -f1)"
echo "pinned:"
grep -E "^Package: (weston|foot|fonts-dejavu-core|xkb-data|seatd|libseat1|libinput10|libxkbcommon0) " "$OUT/VERSIONS.txt" || true

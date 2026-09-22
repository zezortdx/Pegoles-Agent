#!/usr/bin/env bash
# Reproducible Debian package bundle for Pegoles Guest Image v2.
# BUILD TOOLING ONLY (Docker + official apt). The runtime stays offline:
# Debian's apt verifies signed InRelease metadata inside the disposable
# container; only the resulting verified .debs land on the seed ISO.
#
# Usage: bash scripts/build-guest-image/build-deb-bundle.sh [OUT_DIR]
# Output: $OUT_DIR/*.deb + VERSIONS.txt (pinned name/version/arch).
set -euo pipefail
OUT="${1:-/tmp/pgv2-debs}"
mkdir -p "$OUT"

# Pinned minimal graphical stack (trixie arm64, 2026-09). Chromium is
# NOT included (Phase 5.1 needs no browser). Versions from apt policy;
# bump deliberately with a manifest note, never floating.
TOP_PKGS="weston foot fonts-dejavu-core xkb-data"

docker run --rm --platform linux/arm64 -v "$OUT:/out" debian:trixie-slim bash -c "
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
echo "bundle: $(ls "$OUT"/*.deb | wc -l) debs, $(du -sh "$OUT" | cut -f1)"
echo "pinned:"
grep -E "^(weston|foot|fonts-dejavu-core|xkb-data|seatd|libseat1|libinput10|libxkbcommon0) " "$OUT/VERSIONS.txt" || true

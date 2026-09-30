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

# Minimal graphical stack plus the browser (trixie, 2026-09; arm64 or amd64
# per PEGOLES_GUEST_ARCH; chromium since images 0.4 / x64-0.2, see
# docs/EGRESS.md). chromium comes from trixie-security; it is installed
# --no-install-recommends, so the fonts it needs are listed explicitly and
# chromium-sandbox (a setuid helper) is deliberately NOT installed: the
# browser uses the user-namespace sandbox. ca-certificates is already in
# the base image; it is listed so a moved archive version fails here and
# not in the guest. The top-level packages are pinned
# to exact versions: when the archive has moved on, apt fails instead of
# silently taking a newer build; bump deliberately with a manifest note.
# Their dependency closure is whatever the signed archive serves at build
# time and is recorded, per .deb, in VERSIONS.txt (sealed with the image).
# chromium 150.x is the trixie-security build as of 2026-09-30; security
# updates move it often (apt-cache policy chromium), so bump it deliberately
# together with the manifest.
TOP_PKGS="weston=14.0.2-1 foot=1.21.0-2 fonts-dejavu-core=2.37-8 xkb-data=2.42-1 \
  chromium=150.0.7871.181-1~deb13u1 fonts-liberation=1:2.1.5-3 ca-certificates=20250419"
# Guard the two decisions above against an archive that pulls them back in.
FORBIDDEN_PKGS="chromium-sandbox"
# TODO(0.4): the new package manifests (manifests/pegoles-base-0.4.packages.txt,
# pegoles-base-x64-0.2.packages.txt) are produced by the image build from the
# sealed disk's dpkg status; commit them when the images are built.

docker run --rm --platform "$DOCKER_PLATFORM" -v "$OUT:/out" "$DEBIAN_IMAGE" bash -c "
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
grep -E "^Package: (weston|foot|fonts-dejavu-core|xkb-data|seatd|libseat1|libinput10|libxkbcommon0|chromium|chromium-common|fonts-liberation|ca-certificates) " "$OUT/VERSIONS.txt" || true
if grep -qE "^Package: ($FORBIDDEN_PKGS) " "$OUT/VERSIONS.txt"; then
  echo "forbidden package in the bundle: $FORBIDDEN_PKGS" >&2
  exit 1
fi

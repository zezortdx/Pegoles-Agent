#!/usr/bin/env bash
# Cross-compile pegoles-guest-runtime for the guest (aarch64 or x86_64 Linux,
# PEGOLES_GUEST_ARCH; BUILD TIME ONLY).
# Uses Docker (rust image, native arm64 on Apple Silicon) so the macOS app
# never needs a cross toolchain, QEMU, or libguestfs. The end user needs
# none of this: they boot the derived image we publish.
#
# usage: build-runtime.sh [out-dir]   (default: a fresh private temp dir)
# The repository is mounted read-only: the build writes only to its
# in-container target dir and to the output dir.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/build-guest-image/common.sh
. "$ROOT/scripts/build-guest-image/common.sh"
OUT="$(out_dir "${1:-}")"

docker run --rm --platform "$DOCKER_PLATFORM" \
  -v "$ROOT:/work:ro" -v "$OUT:/out" -w /work "$RUST_IMAGE" bash -c "
    cargo build -p pegoles-guest-runtime --release --locked \
      --target $RUST_TARGET --target-dir /tmp/gt &&
    cp /tmp/gt/$RUST_TARGET/release/pegoles-guest-runtime /out/ &&
    /out/pegoles-guest-runtime --version
  "
echo "runtime staged at $OUT/pegoles-guest-runtime"

#!/usr/bin/env bash
# Cross-compile pegoles-guest-runtime for the guest (aarch64 or x86_64 Linux,
# PEGOLES_GUEST_ARCH; BUILD TIME ONLY).
# Uses Docker (rust image, native arm64 on Apple Silicon) so the macOS app
# never needs a cross toolchain, QEMU, or libguestfs. The end user needs
# none of this: they boot the derived image we publish.
#
# Also builds pegoles-egress-forwarder (guest end of the egress channel,
# images 0.4 / x64-0.2). TODO(egress): package and binary name assumed to be
# pegoles-egress-forwarder in the guest/runtime workspace; adjust here if the
# crate is named differently.
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
    cargo build -p pegoles-guest-runtime -p pegoles-egress-forwarder --release --locked \
      --target $RUST_TARGET --target-dir /tmp/gt &&
    cp /tmp/gt/$RUST_TARGET/release/pegoles-guest-runtime /tmp/gt/$RUST_TARGET/release/pegoles-egress-forwarder /out/ &&
    /out/pegoles-guest-runtime --version
  "
echo "runtime staged at $OUT/pegoles-guest-runtime (+ pegoles-egress-forwarder)"

#!/usr/bin/env bash
# Cross-compile pegoles-guest-runtime for aarch64 Linux (BUILD TIME ONLY).
# Uses Docker (rust image, native arm64 on Apple Silicon) so the macOS app
# never needs a cross toolchain, QEMU, or libguestfs. The end user needs
# none of this: they boot the derived image we publish.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-/tmp/pgbuild}"

docker run --rm --platform linux/arm64 \
  -v "$ROOT:/work" -v "$OUT:/out" -w /work rust:1.89-bookworm bash -c "
    cargo build -p pegoles-guest-runtime --release --locked \
      --target aarch64-unknown-linux-gnu --target-dir /tmp/gt &&
    cp /tmp/gt/aarch64-unknown-linux-gnu/release/pegoles-guest-runtime /out/ &&
    /out/pegoles-guest-runtime --version
  "
echo "runtime staged at $OUT/pegoles-guest-runtime"

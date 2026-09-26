#!/usr/bin/env bash
# Cross-compile pegoles-input-fixture for aarch64 Linux (BUILD TIME ONLY).
# Same Docker model as build-runtime.sh: the macOS app never needs a
# cross toolchain. The guest never builds anything.
#
# usage: build-fixture.sh [out-dir]   (default: a fresh private temp dir)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/build-guest-image/common.sh
. "$ROOT/scripts/build-guest-image/common.sh"
OUT="$(out_dir "${1:-}")"

docker run --rm --platform linux/arm64 \
  -v "$ROOT:/work:ro" -v "$OUT:/out" -w /work "$RUST_IMAGE" bash -c "
    cargo build --manifest-path guest/fixture/Cargo.toml --release --locked \
      --target aarch64-unknown-linux-gnu --target-dir /tmp/gf &&
    cp /tmp/gf/aarch64-unknown-linux-gnu/release/pegoles-input-fixture /out/ &&
    file /out/pegoles-input-fixture
  "
echo "fixture staged at $OUT/pegoles-input-fixture"

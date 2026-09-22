#!/usr/bin/env bash
# Cross-compile pegoles-input-fixture for aarch64 Linux (BUILD TIME ONLY).
# Same Docker model as build-runtime.sh: the macOS app never needs a
# cross toolchain. The guest never builds anything.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-/tmp/pgbuild}"

docker run --rm --platform linux/arm64 \
  -v "$ROOT:/work" -v "$OUT:/out" -w /work rust:1.89-bookworm bash -c "
    cargo build --manifest-path guest/fixture/Cargo.toml --release \
      --target aarch64-unknown-linux-gnu --target-dir /tmp/gf &&
    cp /tmp/gf/aarch64-unknown-linux-gnu/release/pegoles-input-fixture /out/ &&
    file /out/pegoles-input-fixture
  "
echo "fixture staged at $OUT/pegoles-input-fixture"

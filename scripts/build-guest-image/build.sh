#!/usr/bin/env bash
# Build the Pegoles Base Image v0.1 (BUILD TIME ONLY).
#
#   official Debian 13 generic ARM64 (verified)
#     -> boot once with NoCloud seed ISO (runtime binary + cloud-init)
#     -> cloud-init installs runtime + unit, disables itself, poweroff
#     -> seal disk as images/pegoles-base-0.1 (+ manifest + sha512)
#
# The macOS app never runs this: it boots per-computer copies of the
# derived image. No QEMU, libguestfs, or Docker needed at runtime.
#
# usage: build.sh [work-dir]   (default: a fresh private temp dir)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/build-guest-image/common.sh
. "$ROOT/scripts/build-guest-image/common.sh"
OUT="$(out_dir "${1:-}")"

bash "$ROOT/scripts/build-guest-image/build-runtime.sh" "$OUT"
bash "$ROOT/scripts/build-guest-image/make-seed-iso.sh" \
  "$OUT/pegoles-guest-runtime" "$OUT/seed.iso"

PEGOLES_SEED_ISO="$OUT/seed.iso" PEGOLES_IMAGE_SPEC=generic \
  cargo run -p pegoles-computer --example provision_base_image

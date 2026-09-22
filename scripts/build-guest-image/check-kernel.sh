#!/usr/bin/env bash
# Validate vsock transport support in a Pegoles guest disk image.
# BUILD TIME ONLY. Mounts the raw image in a throwaway Linux container
# (needs Docker on the build machine; never on the end-user machine),
# extracts /boot/config-*, and runs the Rust checker. Fails closed:
# a future universal image must provide BOTH virtio and Hyper-V vsock.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DISK="${1:?usage: check-kernel.sh <disk.raw> [--strict-universal]}"

OFFSET=$((262144 * 512))
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

docker run --rm --privileged --platform linux/arm64 \
  -v "$DISK:/img/disk.raw:ro" -v "$WORK:/out" ubuntu:24.04 bash -c "
    mkdir -p /mnt/root &&
    mount -o ro,loop,offset=$OFFSET /img/disk.raw /mnt/root &&
    ls /mnt/root/boot/config-* > /dev/null &&
    cp \$(ls /mnt/root/boot/config-* | head -1) /out/kernel-config &&
    umount /mnt/root
  "

cargo run -q -p pegoles-computer --example check_guest_kernel -- "$WORK/kernel-config" ${2:-}

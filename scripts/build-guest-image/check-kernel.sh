#!/usr/bin/env bash
# Validate vsock transport support in a Pegoles guest disk image.
# BUILD TIME ONLY. Reads /boot/config-* straight out of the ext4 root
# filesystem with debugfs (e2fsprogs, as patch-image.sh does): read-only,
# no container, no loop device, no privileges. Then runs the Rust
# checker. Fails closed: a future universal image must provide BOTH
# virtio and Hyper-V vsock.
set -euo pipefail
DISK="${1:?usage: check-kernel.sh <disk.raw> [--strict-universal]}"
E2FS="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DEBUGFS="$E2FS/debugfs"
[ -x "$DEBUGFS" ] || { echo "need e2fsprogs (brew install e2fsprogs)" >&2; exit 1; }
[ -f "$DISK" ] || { echo "missing disk: $DISK" >&2; exit 1; }

OFFSET=$((262144 * 512))
FS="$DISK?offset=$OFFSET"
# debugfs splits its command lines on whitespace; mktemp paths have none.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `ls -p` prints /inode/mode/uid/gid/name/size/ per entry.
CONFIG="$("$DEBUGFS" -R "ls -p /boot" "$FS" 2>/dev/null \
  | awk -F/ '$6 ~ /^config-/ { print $6 }' | LC_ALL=C sort | sed -n 1p)"
[ -n "$CONFIG" ] || { echo "no /boot/config-* in the root filesystem of $DISK" >&2; exit 1; }
"$DEBUGFS" -R "dump /boot/$CONFIG $WORK/kernel-config" "$FS" 2>/dev/null
[ -s "$WORK/kernel-config" ] || { echo "could not read /boot/$CONFIG" >&2; exit 1; }

cargo run -q -p pegoles-computer --example check_guest_kernel -- "$WORK/kernel-config" ${2:+"$2"}

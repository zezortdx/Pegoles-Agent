#!/usr/bin/env bash
# Offline-enable cloud-init.target on a provision disk (BUILD TIME ONLY).
#
# The Debian nocloud generic image ships cloud-init INSTALLED but with
# nothing pulling cloud-init.target, so a seed ISO alone never triggers
# provisioning (found by Phase 5.1 hardware forensics: a builder VM
# idles forever waiting for a poweroff that never comes). This script
# creates the single enablement symlink offline:
#
#   /etc/systemd/system/multi-user.target.wants/cloud-init.target
#     -> /lib/systemd/system/cloud-init.target
#
# Method: host e2fsprogs on the root filesystem at its byte offset
# (debugfs/e2fsck `<disk>?offset=`, as patch-image.sh does): journal
# replay, symlink, fsck-verify. No container, no nbd/loop device, no
# privileges, no packages installed at build time.
# Idempotent: exits 0 immediately if the symlink already exists.
#
# Usage: enable-cloud-init.sh <disk.img> [partition-offset-bytes]
#   Offset defaults to the generic image root partition (134217728).
set -euo pipefail
DISK="${1:?usage: enable-cloud-init.sh <disk.img> [offset]}"
OFFSET="${2:-134217728}"
E2FS="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DEBUGFS="$E2FS/debugfs"
E2FSCK="$E2FS/e2fsck"
[ -x "$DEBUGFS" ] && [ -x "$E2FSCK" ] || { echo "need e2fsprogs (brew install e2fsprogs)" >&2; exit 1; }
[ -f "$DISK" ] || { echo "missing disk: $DISK" >&2; exit 1; }
case "$OFFSET" in '' | *[!0-9]*) echo "offset must be a byte count: $OFFSET" >&2; exit 1 ;; esac

FS="$DISK?offset=$OFFSET"
WANTS=/etc/systemd/system/multi-user.target.wants
# grep reads all input (no -q): an early exit would SIGPIPE the pipeline.
has_link() { "$DEBUGFS" -R "ls -l $WANTS" "$FS" 2>/dev/null | tr -s ' \t' '\n' | grep -x 'cloud-init.target' >/dev/null; }

if has_link; then
  echo "already enabled"
  exit 0
fi
"$E2FSCK" -y -f "$FS" >/dev/null 2>&1 || true
"$DEBUGFS" -w -R "symlink $WANTS/cloud-init.target /lib/systemd/system/cloud-init.target" "$FS"
has_link || { echo "symlink not present after write" >&2; exit 1; }
"$E2FSCK" -n -f "$FS" 2>&1 | tail -1
echo "cloud-init enabled on $DISK"

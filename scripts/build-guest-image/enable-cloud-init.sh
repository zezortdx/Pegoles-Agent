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
# Method: privileged Docker (nbd + debugfs + e2fsck, all BUILD deps —
# never runtime deps): journal replay, symlink, fsck-verify, detach.
# Idempotent: exits 0 immediately if the symlink already exists.
#
# Usage: enable-cloud-init.sh <disk.img> [partition-offset-bytes]
#   Offset defaults to the generic image root partition (134217728).
set -euo pipefail
DISK="${1:?usage: enable-cloud-init.sh <disk.img> [offset]}"
OFFSET="${2:-134217728}"
NBD="${NBD_DEV:-/dev/nbd0}"

docker run --rm --privileged --platform linux/arm64 -v "$DISK:/d/disk.img:rw" debian:trixie-slim bash -c "
  set -euo pipefail
  apt-get update -q >/dev/null 2>&1 && apt-get install -y -q qemu-utils e2fsprogs >/dev/null 2>&1
  qemu-nbd --disconnect $NBD 2>/dev/null || true
  sleep 1
  qemu-nbd -f raw -c $NBD --offset=$OFFSET /d/disk.img
  sleep 2
  LINK=\$(debugfs -R 'ls -l /etc/systemd/system/multi-user.target.wants' $NBD 2>/dev/null | tr ' ' '\n' | grep -c cloud-init.target || true)
  if [ \"\$LINK\" -ge 1 ]; then echo 'already enabled'; qemu-nbd --disconnect $NBD; exit 0; fi
  e2fsck -y -f $NBD >/dev/null 2>&1 || true
  debugfs -w -R 'symlink /etc/systemd/system/multi-user.target.wants/cloud-init.target /lib/systemd/system/cloud-init.target' $NBD
  debugfs -R 'ls -l /etc/systemd/system/multi-user.target.wants' $NBD | tr ' ' '\n' | grep cloud-init.target
  e2fsck -n -f $NBD 2>&1 | tail -1
  qemu-nbd --disconnect $NBD
"
echo "cloud-init enabled on $DISK"

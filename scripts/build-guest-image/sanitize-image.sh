#!/usr/bin/env bash
# Remove per-instance secrets and state from a Pegoles work disk before it
# is sealed for publication, WITHOUT booting it (debugfs). BUILD TIME ONLY.
#
# Provisioning boots the image once (cloud-init), which generates state that
# would otherwise be identical in every user's computer and published with
# the release: SSH host private keys (sshd is masked and the VM has no
# network, but private keys have no place in a public artifact) and
# systemd's random seed (the guest gets fresh entropy from virtio-rng).
# The script then proves that no private key remains in the usual places.
#
#   usage: sanitize-image.sh <work-disk.img>     (then seal_image)
set -euo pipefail
IMG="${1:?usage: sanitize-image.sh <work-disk.img>}"
E2FS="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DEBUGFS="$E2FS/debugfs"
E2FSCK="$E2FS/e2fsck"
[ -x "$DEBUGFS" ] && [ -x "$E2FSCK" ] || { echo "need e2fsprogs (brew install e2fsprogs)" >&2; exit 1; }
[ -f "$IMG" ] || { echo "missing $IMG" >&2; exit 1; }

# Largest GPT partition = Linux root.
OFFSET="$(python3 - "$IMG" <<'PY'
import struct, sys
with open(sys.argv[1], 'rb') as f:
    f.seek(512)
    hdr = f.read(92)
    assert hdr[:8] == b'EFI PART', 'not a GPT disk'
    lba, num, size = struct.unpack('<QII', hdr[72:88])
    f.seek(lba * 512)
    best = (0, 0)
    for _ in range(num):
        e = f.read(size)
        first, last = struct.unpack('<QQ', e[32:48])
        if first and last - first > best[1] - best[0]:
            best = (first, last)
    print(best[0] * 512)
PY
)"
FS="$IMG?offset=$OFFSET"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
CMDS="$STAGE/cmds"
: > "$CMDS"
for k in rsa ecdsa ed25519 dsa; do
  echo "rm /etc/ssh/ssh_host_${k}_key" >> "$CMDS"
  echo "rm /etc/ssh/ssh_host_${k}_key.pub" >> "$CMDS"
done
echo "rm /var/lib/systemd/random-seed" >> "$CMDS"
"$DEBUGFS" -w -f "$CMDS" "$FS" 2>&1 \
  | grep -vE "^debugfs( [0-9]|: rm )|File not found by ext2_lookup|^$" || true
"$E2FSCK" -fn "$FS" >/dev/null

# Proof: nothing that looks like a private key in the places that hold them.
fail=0
if "$DEBUGFS" -R "ls /etc/ssh" "$FS" 2>/dev/null | grep -q "ssh_host_"; then
  echo "FAIL: SSH host keys still present" >&2; fail=1
fi
if "$DEBUGFS" -R "stat /var/lib/systemd/random-seed" "$FS" 2>/dev/null | grep -q "Inode:"; then
  echo "FAIL: random seed still present" >&2; fail=1
fi
for dir in /etc/ssh /etc/ssl/private /root /root/.ssh /home/debian /home/debian/.ssh /var/lib/pegoles; do
  names="$("$DEBUGFS" -R "ls -p $dir" "$FS" 2>/dev/null | awk -F/ 'NF>5 && $3 ~ /^100/ {print $6}')" || true
  for name in $names; do
    if "$DEBUGFS" -R "cat $dir/$name" "$FS" 2>/dev/null | grep -q "PRIVATE KEY"; then
      echo "FAIL: private key material in $dir/$name" >&2; fail=1
    fi
  done
done
for f in /root/.ssh/authorized_keys /home/debian/.ssh/authorized_keys; do
  size="$("$DEBUGFS" -R "stat $f" "$FS" 2>/dev/null | sed -n 's/.*Size: \([0-9]*\).*/\1/p' | head -1)"
  if [ -n "$size" ] && [ "$size" != "0" ]; then
    echo "FAIL: $f is not empty" >&2; fail=1
  fi
done
[ "$fail" = 0 ] || exit 1
echo "sanitized: no SSH host keys, no random seed, no private keys or authorized keys"

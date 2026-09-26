#!/usr/bin/env bash
# Produce a new Pegoles image work disk from an already-provisioned one,
# WITHOUT booting it: write the current guest runtime + unit files from
# this repo into the root filesystem with debugfs, set the enabled
# services, then fsck. BUILD TIME ONLY.
#
# The same unit files (seed/units/*.service) feed the full cloud-init
# build (make-seed-iso.sh + user-data), so both paths produce the same
# guest configuration. Seal the result with:
#   PEGOLES_DEBS_DIR=<debs> cargo run -p pegoles-computer --example seal_image -- <out.img>
#
# usage: patch-image.sh <provisioned-disk.img> <runtime-bin> <out.img>
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UNITS="$ROOT/scripts/build-guest-image/seed/units"
SRC="${1:?usage: patch-image.sh <provisioned-disk.img> <runtime-bin> <out.img>}"
RUNTIME="${2:?runtime binary}"
OUT="${3:?output image}"
E2FS="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DEBUGFS="$E2FS/debugfs"
E2FSCK="$E2FS/e2fsck"
[ -x "$DEBUGFS" ] && [ -x "$E2FSCK" ] || { echo "need e2fsprogs (brew install e2fsprogs)"; exit 1; }
[ -f "$SRC" ] && [ -f "$RUNTIME" ] || { echo "missing input"; exit 1; }
[ "$SRC" != "$OUT" ] || { echo "refusing to patch the source disk in place"; exit 1; }

# Copy-on-write clone on APFS (instant, no extra space until written).
rm -f "$OUT"
cp -c "$SRC" "$OUT" 2>/dev/null || cp "$SRC" "$OUT"

# Largest GPT partition = Linux root (the EFI partition is ~127 MiB).
OFFSET="$(python3 - "$OUT" <<'PY'
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
FS="$OUT?offset=$OFFSET"
echo "root filesystem at byte offset $OFFSET"

# debugfs splits its command lines on whitespace: stage every input in a
# space-free temp dir first (the repo path may contain spaces).
STAGE="$(mktemp -d)"
CMDS="$STAGE/cmds"
trap 'rm -rf "$STAGE"' EXIT
cp "$RUNTIME" "$STAGE/pegoles-guest-runtime"
mkdir "$STAGE/units"
cp "$UNITS"/*.service "$STAGE/units/"
put() { # put <local> <guest path> <mode>
  {
    echo "rm $2"
    echo "write $1 $2"
    echo "set_inode_field $2 mode 0100$3"
    echo "set_inode_field $2 uid 0"
    echo "set_inode_field $2 gid 0"
  } >> "$CMDS"
}
put "$STAGE/pegoles-guest-runtime" /usr/local/bin/pegoles-guest-runtime 755
for unit in "$STAGE"/units/*.service; do
  put "$unit" "/etc/systemd/system/$(basename "$unit")" 644
done

WANTS=/etc/systemd/system/multi-user.target.wants
enable() { # enable <unit>
  echo "rm $WANTS/$1" >> "$CMDS"
  echo "symlink $WANTS/$1 /etc/systemd/system/$1" >> "$CMDS"
}
enable pegoles-guest-runtime.service
enable pegoles-workspace.service
# Dev fixture: installed, not enabled in the product image.
echo "rm $WANTS/pegoles-fixture.service" >> "$CMDS"
# No NIC exists in a Pegoles computer: network, remote-login and update
# services are dead weight and attack surface. Mask them (a mask also
# beats socket/dbus activation), and mask systemd's ssh generator, which
# would otherwise expose sshd on AF_VSOCK and a local AF_UNIX socket.
mask() { # mask <path under /etc/systemd>
  echo "rm /etc/systemd/$1" >> "$CMDS"
  echo "symlink /etc/systemd/$1 /dev/null" >> "$CMDS"
}
echo "mkdir /etc/systemd/system-generators" >> "$CMDS"
mask system-generators/systemd-ssh-generator
for unit in ssh.service ssh.socket systemd-networkd.service systemd-networkd.socket \
  systemd-networkd-wait-online.service systemd-resolved.service systemd-timesyncd.service \
  unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer apt-listchanges.timer \
  man-db.timer dpkg-db-backup.timer; do
  mask "system/$unit"
done
for dead in \
  multi-user.target.wants/ssh.service \
  multi-user.target.wants/unattended-upgrades.service \
  multi-user.target.wants/systemd-networkd.service \
  sockets.target.wants/systemd-networkd.socket \
  network-online.target.wants/systemd-networkd-wait-online.service \
  sysinit.target.wants/systemd-resolved.service \
  sysinit.target.wants/systemd-timesyncd.service \
  timers.target.wants/apt-daily.timer \
  timers.target.wants/apt-daily-upgrade.timer \
  timers.target.wants/apt-listchanges.timer \
  timers.target.wants/man-db.timer \
  timers.target.wants/dpkg-db-backup.timer; do
  echo "rm /etc/systemd/system/$dead" >> "$CMDS"
done

"$DEBUGFS" -w -f "$CMDS" "$FS" 2>&1 \
  | grep -vE "^debugfs( [0-9]|: (rm|write|set_inode_field|symlink|mkdir) )|File not found by ext2_lookup|^Allocated inode|^$" || true
echo "== fsck"
"$E2FSCK" -fn "$FS"
echo "== verify"
"$DEBUGFS" -R "ls $WANTS" "$FS" 2>/dev/null | tr -s ' ' '\n' | grep -E '\.service$' | sort
"$DEBUGFS" -R "stat /usr/local/bin/pegoles-guest-runtime" "$FS" 2>/dev/null | grep -E "Mode|User|Size:" | head -3
for unit in "$STAGE"/units/*.service; do
  name="$(basename "$unit")"
  got="$("$DEBUGFS" -R "cat /etc/systemd/system/$name" "$FS" 2>/dev/null)"
  [ "$got" = "$(cat "$unit")" ] || { echo "FAIL: $name content mismatch"; exit 1; }
done
echo "unit files verified byte-for-byte"
echo "patched image at $OUT"

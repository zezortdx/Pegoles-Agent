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
# Images 0.4 / x64-0.2 (browser): also writes the forwarder binary
# (PEGOLES_FORWARDER_BIN, required), the managed Chromium policy tree, the
# browser units, the panel launcher and weston.ini. debugfs cannot install
# packages or create users: the SOURCE disk must already have been
# provisioned with chromium and the pegoles-egress user (the cloud-init
# boot of seed/user-data does both); this script checks that and refuses
# otherwise. PEGOLES_PATCH_NO_BROWSER=1 skips all browser steps (a 0.3-style
# refresh of a disk without a browser).
#
# usage: patch-image.sh <provisioned-disk.img> <runtime-bin> <out.img>
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SEED="$ROOT/scripts/build-guest-image/seed"
UNITS="$SEED/units"
BROWSER=1
[ "${PEGOLES_PATCH_NO_BROWSER:-0}" != 1 ] || BROWSER=0
SRC="${1:?usage: patch-image.sh <provisioned-disk.img> <runtime-bin> <out.img>}"
RUNTIME="${2:?runtime binary}"
OUT="${3:?output image}"
E2FS="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DEBUGFS="$E2FS/debugfs"
E2FSCK="$E2FS/e2fsck"
[ -x "$DEBUGFS" ] && [ -x "$E2FSCK" ] || { echo "need e2fsprogs (brew install e2fsprogs)"; exit 1; }
[ -f "$SRC" ] && [ -f "$RUNTIME" ] || { echo "missing input"; exit 1; }
if [ "$BROWSER" = 1 ]; then
  FORWARDER="${PEGOLES_FORWARDER_BIN:?set PEGOLES_FORWARDER_BIN (pegoles-egress-forwarder) or PEGOLES_PATCH_NO_BROWSER=1}"
  [ -f "$FORWARDER" ] || { echo "missing forwarder binary $FORWARDER"; exit 1; }
  for j in "$SEED"/browser/pegoles-policy.json "$SEED"/browser/pegoles-egress-ca.json; do
    python3 -m json.tool "$j" >/dev/null || { echo "invalid JSON: $j"; exit 1; }
  done
fi
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
cp "$UNITS"/*.service "$UNITS"/*.path "$STAGE/units/"
cp "$ROOT/scripts/build-guest-image/seed/sysctl/60-pegoles-hardening.conf" "$STAGE/sysctl.conf"
put() { # put <local> <guest path> <mode> [uid gid]
  {
    echo "rm $2"
    echo "write $1 $2"
    echo "set_inode_field $2 mode 0100$3"
    echo "set_inode_field $2 uid ${4:-0}"
    echo "set_inode_field $2 gid ${5:-0}"
  } >> "$CMDS"
}
mkdir_root() { # mkdir_root <guest dir> <mode, e.g. 755>: root-owned directory
  {
    echo "mkdir $1"
    echo "set_inode_field $1 mode 040$2"
    echo "set_inode_field $1 uid 0"
    echo "set_inode_field $1 gid 0"
  } >> "$CMDS"
}
put "$STAGE/pegoles-guest-runtime" /usr/local/bin/pegoles-guest-runtime 755
for unit in "$STAGE"/units/*.service "$STAGE"/units/*.path; do
  put "$unit" "/etc/systemd/system/$(basename "$unit")" 644
done

put "$STAGE/sysctl.conf" /etc/sysctl.d/60-pegoles-hardening.conf 644

if [ "$BROWSER" = 1 ]; then
  # Preconditions (read-only): provisioning must already have installed the
  # browser and created the forwarder's user.
  "$DEBUGFS" -R "stat /usr/bin/chromium" "$FS" >/dev/null 2>&1 \
    || { echo "FAIL: /usr/bin/chromium missing: the source disk was not provisioned with the browser (run the full build)"; exit 1; }
  "$DEBUGFS" -R "stat /usr/share/icons/hicolor/48x48/apps/chromium.png" "$FS" >/dev/null 2>&1 \
    || { echo "FAIL: chromium icon missing (panel launcher would be blank)"; exit 1; }
  EGRESS_IDS="$("$DEBUGFS" -R "cat /etc/passwd" "$FS" 2>/dev/null | awk -F: '$1=="pegoles-egress"{print $3" "$4}')"
  [ -n "$EGRESS_IDS" ] || { echo "FAIL: user pegoles-egress missing in the source disk"; exit 1; }
  # shellcheck disable=SC2086 # "uid gid"
  set -- $EGRESS_IDS
  EGRESS_UID="$1" EGRESS_GID="$2"
  [ "$EGRESS_UID" != 0 ] && [ "$EGRESS_GID" != 0 ] || { echo "FAIL: pegoles-egress must not be uid/gid 0"; exit 1; }

  # Managed-policy tree: every directory root:root 0755, pegoles.json root
  # 0644, and the one file the forwarder may write owned by pegoles-egress.
  for d in /etc/chromium /etc/chromium/policies /etc/chromium/policies/managed /usr/local/lib/pegoles; do
    mkdir_root "$d" 755
  done
  put "$SEED/browser/pegoles-policy.json" /etc/chromium/policies/managed/pegoles.json 644
  put "$SEED/browser/pegoles-egress-ca.json" /etc/chromium/policies/managed/pegoles-egress-ca.json 644 "$EGRESS_UID" "$EGRESS_GID"
  put "$FORWARDER" /usr/local/lib/pegoles/pegoles-egress-forwarder 755
  put "$SEED/browser/pegoles-open-browser" /usr/local/bin/pegoles-open-browser 755
  put "$SEED/weston/weston.ini" /etc/xdg/weston/weston.ini 644
fi

WANTS=/etc/systemd/system/multi-user.target.wants
enable() { # enable <unit>
  echo "rm $WANTS/$1" >> "$CMDS"
  echo "symlink $WANTS/$1 /etc/systemd/system/$1" >> "$CMDS"
}
enable pegoles-guest-runtime.service
enable pegoles-workspace.service
if [ "$BROWSER" = 1 ]; then
  enable pegoles-egress-forwarder.service
  enable pegoles-browser.path
  # pegoles-browser.service has no [Install]: only the path unit starts it.
else
  echo "rm $WANTS/pegoles-egress-forwarder.service" >> "$CMDS"
  echo "rm $WANTS/pegoles-browser.path" >> "$CMDS"
fi
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
for unit in "$STAGE"/units/*.service "$STAGE"/units/*.path; do
  name="$(basename "$unit")"
  got="$("$DEBUGFS" -R "cat /etc/systemd/system/$name" "$FS" 2>/dev/null)"
  [ "$got" = "$(cat "$unit")" ] || { echo "FAIL: $name content mismatch"; exit 1; }
done
echo "unit files verified byte-for-byte"
if [ "$BROWSER" = 1 ]; then
  # Ownership/mode gate: the agent's user (pegoles) must not be able to write
  # anything in the policy tree. Expected "mode uid gid" per path.
  owner_check() { # owner_check <path> <d|f (informational)> <perm> <uid> <gid>
    local got
    got="$("$DEBUGFS" -R "stat $1" "$FS" 2>/dev/null | awk '/Mode:/{m=$0; sub(/.*Mode: +/,"",m); sub(/ .*/,"",m)} /User:/{u=$2; g=$4} END{print m" "u" "g}')"
    [ "$got" = "$3 $4 $5" ] || { echo "FAIL: $1 is [$got], expected [$3 $4 $5]"; exit 1; }
  }
  for d in /etc/chromium /etc/chromium/policies /etc/chromium/policies/managed; do owner_check "$d" d 0755 0 0; done
  owner_check /etc/chromium/policies/managed/pegoles.json f 0644 0 0
  owner_check /etc/chromium/policies/managed/pegoles-egress-ca.json f 0644 "$EGRESS_UID" "$EGRESS_GID"
  owner_check /usr/local/lib/pegoles/pegoles-egress-forwarder f 0755 0 0
  # Nothing else may live in the managed dir (Chromium loads every *.json).
  left="$("$DEBUGFS" -R "ls -l /etc/chromium/policies/managed" "$FS" 2>/dev/null | awk '{print $NF}' | grep -vE '^(\.|\.\.|pegoles\.json|pegoles-egress-ca\.json)$' | grep . || true)"
  [ -z "$left" ] || { echo "FAIL: unexpected files in the managed policy dir: $left"; exit 1; }
  echo "browser policy tree, forwarder, launcher verified"
fi
echo "patched image at $OUT"

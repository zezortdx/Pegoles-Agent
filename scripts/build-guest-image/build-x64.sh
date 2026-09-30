#!/usr/bin/env bash
# Build the x64 Pegoles image for Windows (Hyper-V). BUILD TIME ONLY.
#
# Provisioned here, once, exactly like the arm64 image — never on the
# user's PC (only a sealed image boots in normal flows):
#
#   official Debian 13 generic amd64 (dated build, SHA-512 pinned below)
#     -> NoCloud seed: x86_64 guest runtime + input fixture, the pinned
#        amd64 .deb bundle, the same user-data and units as arm64
#     -> booted once under QEMU (KVM on Linux, TCG elsewhere; no network):
#        cloud-init installs everything, applies the Hyper-V specifics
#        (listen mode, COM1 console, hv_sock, initramfs drivers; see
#        seed/user-data), disables itself and powers off
#     -> patch-image.sh (runtime, units, masked services; no boot)
#     -> sanitize-image.sh (host keys, random seed)
#     -> disk.vhdx (dynamic) + manifest.json (SHA-512 of every artifact)
#        + packages.txt (every installed package and version)
#
# usage: build-x64.sh [out-dir]   (default: a fresh private temp dir)
#
# Needs: docker, qemu-system-x86_64 and qemu-img, an x86_64 UEFI firmware
# (QEMU's edk2-x86_64-code.fd or Debian's OVMF), e2fsprogs (debugfs,
# e2fsck), xz, curl, python3, and hdiutil (macOS) or xorriso (Linux).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/scripts/build-guest-image"
export PEGOLES_GUEST_ARCH=amd64
# shellcheck source=scripts/build-guest-image/common.sh
. "$HERE/common.sh"
OUT="$(out_dir "${1:-}")"

SOURCE_URL="https://cdimage.debian.org/images/cloud/trixie/20260914-2601/debian-13-generic-amd64-20260914-2601.tar.xz"
SOURCE_SHA512="40017fab9e1ed4bd3d2068e49301ac8427d02c2ee418160bdedd041f529e048fa0dc8ddf8f363e85bdb3c33e9f3cf4c5734d98429ba6802bbc4d4a83df7f0f4b"
# Provisioning takes minutes with KVM and much longer under TCG.
PROVISION_TIMEOUT_S="${PEGOLES_PROVISION_TIMEOUT_S:-5400}"

sha512() { if command -v sha512sum >/dev/null; then sha512sum "$1"; else shasum -a 512 "$1"; fi | cut -d' ' -f1; }

if [ -z "${E2FSPROGS:-}" ]; then
  for d in /opt/homebrew/opt/e2fsprogs/sbin /usr/sbin /sbin; do
    [ -x "$d/debugfs" ] && { E2FSPROGS="$d"; break; }
  done
fi
export E2FSPROGS="${E2FSPROGS:?e2fsprogs (debugfs) not found}"

OVMF=""
for f in "${OVMF_CODE:-}" /opt/homebrew/share/qemu/edk2-x86_64-code.fd /usr/share/qemu/edk2-x86_64-code.fd \
  /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
  [ -n "$f" ] && [ -f "$f" ] && { OVMF="$f"; break; }
done
[ -n "$OVMF" ] || { echo "x86_64 UEFI firmware not found (set OVMF_CODE)" >&2; exit 1; }

# --- 1. the official source, verified -------------------------------------
if [ ! -f "$OUT/source.tar.xz" ] || [ "$(sha512 "$OUT/source.tar.xz")" != "$SOURCE_SHA512" ]; then
  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error -o "$OUT/source.tar.xz" "$SOURCE_URL"
fi
[ "$(sha512 "$OUT/source.tar.xz")" = "$SOURCE_SHA512" ] || { echo "source image SHA-512 mismatch" >&2; exit 1; }
rm -f "$OUT/disk.raw" "$OUT/work.raw"
tar -xJf "$OUT/source.tar.xz" -C "$OUT" disk.raw
mv "$OUT/disk.raw" "$OUT/work.raw"
echo "source verified; work disk $(wc -c < "$OUT/work.raw" | tr -d ' ') bytes"

# --- 2. guest binaries and packages ------------------------------------------
bash "$HERE/build-runtime.sh" "$OUT/runtime"
# Guest end of the egress channel (images x64-0.2+); picked up by
# make-seed-iso.sh and patch-image.sh.
export PEGOLES_FORWARDER_BIN="$OUT/runtime/pegoles-egress-forwarder"
bash "$HERE/build-fixture.sh" "$OUT/fixture"
bash "$HERE/build-deb-bundle.sh" "$OUT/debs"
bash "$HERE/make-seed-iso.sh" "$OUT/runtime/pegoles-guest-runtime" "$OUT/fixture/pegoles-input-fixture" \
  "$OUT/debs" "$OUT/seed.iso"

# --- 3. provision: one boot, offline ------------------------------------------
ACCEL=tcg
if [ -w /dev/kvm ]; then ACCEL=kvm; fi
echo "provisioning under QEMU ($ACCEL, firmware $OVMF)"
rm -f "$OUT/provision-console.log"
qemu-system-x86_64 -machine q35 -accel "$ACCEL" -cpu max -smp 2 -m 4096 \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF" \
  -drive file="$OUT/work.raw",format=raw,if=virtio \
  -drive file="$OUT/seed.iso",media=cdrom,readonly=on \
  -nic none -display none -serial file:"$OUT/provision-console.log" -no-reboot &
QEMU_PID=$!
waited=0
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if [ "$waited" -ge "$PROVISION_TIMEOUT_S" ]; then
    kill "$QEMU_PID" || true
    tail -40 "$OUT/provision-console.log" >&2 || true
    echo "provisioning did not finish in ${PROVISION_TIMEOUT_S}s" >&2
    exit 1
  fi
  sleep 10
  waited=$((waited + 10))
done
wait "$QEMU_PID" || true
grep -q "Pegoles v2 provisioning complete" "$OUT/provision-console.log" || {
  tail -60 "$OUT/provision-console.log" >&2
  echo "provisioning did not complete (see provision-console.log)" >&2
  exit 1
}
echo "provisioned in ~${waited}s"

# --- 4. patch, sanitize ----------------------------------------------------------
bash "$HERE/patch-image.sh" "$OUT/work.raw" "$OUT/runtime/pegoles-guest-runtime" "$OUT/disk.raw"
rm -f "$OUT/work.raw"
bash "$HERE/sanitize-image.sh" "$OUT/disk.raw"

# Installed packages, for the manifest (read-only, no boot).
OFFSET="$(python3 - "$OUT/disk.raw" <<'PY'
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
"$E2FSPROGS/debugfs" -R "cat /var/lib/dpkg/status" "$OUT/disk.raw?offset=$OFFSET" 2>/dev/null \
  | awk '/^Package: /{p=$2} /^Architecture: /{a=$2} /^Version: /{v=$2} /^$/{if(p)print p" "v" "a; p=""}' \
  | sort > "$OUT/packages.txt"
[ -s "$OUT/packages.txt" ] || { echo "could not read the package list" >&2; exit 1; }
grep -qE '^linux-image-[0-9].* amd64$' "$OUT/packages.txt" || { echo "no amd64 kernel in the image" >&2; exit 1; }
grep -qE '^chromium [^ ]+ amd64$' "$OUT/packages.txt" || { echo "no chromium in the image" >&2; exit 1; }
! grep -qE '^chromium-sandbox ' "$OUT/packages.txt" || { echo "chromium-sandbox (setuid) must not be installed" >&2; exit 1; }
# TODO(x64-0.2): commit packages.txt as manifests/pegoles-base-x64-0.2.packages.txt.

# --- 5. VHDX + manifest ------------------------------------------------------------
rm -f "$OUT/disk.vhdx"
qemu-img convert -f raw -O vhdx -o subformat=dynamic "$OUT/disk.raw" "$OUT/disk.vhdx"
python3 - "$OUT" "$SOURCE_URL" "$SOURCE_SHA512" "$ROOT/guest/runtime/Cargo.toml" <<'PY'
import hashlib, json, os, re, sys
out, url, source_sha, cargo = sys.argv[1:5]
def digest(path):
    h = hashlib.sha512()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()
version = re.search(r'^version = "([^"]+)"', open(cargo).read(), re.M).group(1)
packages = open(os.path.join(out, 'packages.txt')).read().splitlines()
pick = lambda name: next((p.split()[1] for p in packages if p.split()[0] == name), None)
manifest = {
    'architecture': 'amd64',
    'debian_version': '13',
    'guest_runtime_version': version,
    'guest_transport': 'listen (vsock 850, hv_sock)',
    'source': {'url': url, 'sha512': source_sha},
    'graphical': {'compositor': f"weston {pick('weston')}", 'terminal': f"foot {pick('foot')}"},
    'browser': {'chromium': pick('chromium'), 'proxy': '127.0.0.1:3128 (managed policy, docs/EGRESS.md)'},
    'disk_raw': {'bytes': os.path.getsize(os.path.join(out, 'disk.raw')), 'sha512': digest(os.path.join(out, 'disk.raw'))},
    'disk_vhdx': {'bytes': os.path.getsize(os.path.join(out, 'disk.vhdx')), 'sha512': digest(os.path.join(out, 'disk.vhdx'))},
    'packages': len(packages),
    'packages_sha256': hashlib.sha256(open(os.path.join(out, 'packages.txt'), 'rb').read()).hexdigest(),
}
with open(os.path.join(out, 'manifest.json'), 'w') as f:
    json.dump(manifest, f, indent=2)
print(json.dumps(manifest, indent=2))
PY
echo "x64 image at $OUT/disk.vhdx (unsealed build output; see docs/WINDOWS_ARCHITECTURE.md)"

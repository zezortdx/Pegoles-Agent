#!/usr/bin/env bash
# Verify a built "Pegoles Agent.app" (or a DMG containing it) before it is
# distributed. Fails on the first violated property.
#
#   bash scripts/release/verify-artifact.sh <app-or-dmg> [--signed|--distribution]
#
# Always checked: strict signature of the bundle, every Mach-O file signed,
# the helper carries exactly the virtualization entitlement, the app and the
# interpreter carry none, the bundled runtime and worker are present, no
# file is group/other-writable, no symlink escapes the bundle, no
# developer paths, venvs, keys or model weights are inside.
# With --signed additionally: Developer ID authority with a Team ID,
# hardened runtime and secure timestamp on every Mach-O (before
# notarization). With --distribution also: Gatekeeper assessment and a
# stapled notarization ticket (DMG and app).
set -euo pipefail
TARGET="${1:?usage: verify-artifact.sh <app-or-dmg> [--signed|--distribution]}"
DIST=0
SIGNED=0
case "${2:-}" in
  --distribution) DIST=1; SIGNED=1 ;;
  --signed) SIGNED=1 ;;
  "") ;;
  *) echo "unknown option: $2" >&2; exit 2 ;;
esac

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

MOUNT=""
cleanup() { [ -n "$MOUNT" ] && hdiutil detach -quiet "$MOUNT" || true; }
trap cleanup EXIT

if [[ "$TARGET" == *.dmg ]]; then
  if [ "$DIST" = 1 ]; then
    codesign --verify --strict --verbose=2 "$TARGET" >/dev/null 2>&1 || fail "DMG signature"
    spctl --assess --type open --context context:primary-signature -v "$TARGET" 2>&1 \
      | grep -q "accepted" || fail "Gatekeeper rejects the DMG"
    xcrun stapler validate "$TARGET" >/dev/null || fail "DMG has no stapled ticket"
    ok "DMG signed, accepted by Gatekeeper, stapled"
  fi
  MOUNT="$(mktemp -d)"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$TARGET"
  APP="$MOUNT/Pegoles Agent.app"
  [ -L "$MOUNT/Applications" ] || fail "DMG lacks the Applications link"
else
  APP="$TARGET"
fi
[ -d "$APP/Contents" ] || fail "not an app bundle: $APP"

MAIN="$APP/Contents/MacOS/pegoles-desktop"
HELPER="$APP/Contents/MacOS/pegoles-vm-host"
PY="$APP/Contents/Resources/runtime/python/bin/python3.12"
WORKER="$APP/Contents/Resources/workers/mlx/pegoles_mlx_worker.py"
for f in "$MAIN" "$HELPER" "$PY" "$WORKER" "$APP/Contents/Resources/runtime/runtime-manifest.json" \
  "$APP/Contents/Resources/THIRD_PARTY_NOTICES.md" "$APP/Contents/Resources/LICENSE"; do
  [ -f "$f" ] || fail "missing $f"
done
ok "layout (app, helper, runtime, worker, manifest, licenses)"

codesign --verify --deep --strict --verbose=2 "$APP" >/dev/null 2>&1 \
  || { codesign --verify --deep --strict --verbose=2 "$APP"; fail "bundle signature"; }
ok "bundle signature (strict)"

entitlements() { codesign -d --entitlements - --xml "$1" 2>/dev/null || true; }
entitlements "$HELPER" | grep -q "com.apple.security.virtualization" || fail "helper lacks the virtualization entitlement"
[ "$(entitlements "$HELPER" | grep -o '<key>[^<]*</key>' | sort -u | wc -l | tr -d ' ')" = 1 ] \
  || fail "helper has entitlements beyond virtualization: $(entitlements "$HELPER")"
for f in "$MAIN" "$PY"; do
  entitlements "$f" | grep -q '<key>' && fail "unexpected entitlements on $f: $(entitlements "$f")"
done
ok "entitlements (helper: virtualization only; app and interpreter: none)"

TEAM=""
if [ "$SIGNED" = 1 ]; then
  TEAM="$(codesign -dv "$APP" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
  [ -n "$TEAM" ] && [ "$TEAM" != "not set" ] || fail "no Team ID on the app"
  codesign -dv --verbose=4 "$APP" 2>&1 | grep -q "^Authority=Developer ID Application:" \
    || fail "app not signed with a Developer ID Application certificate"
fi

# Mach-O by magic bytes (thin and universal, either byte order).
is_macho() {
  case "$(head -c 4 "$1" 2>/dev/null | xxd -p)" in
    feedface | feedfacf | cefaedfe | cffaedfe | cafebabe | bebafeca) return 0 ;;
    *) return 1 ;;
  esac
}
machos=0
while IFS= read -r -d '' f; do
  is_macho "$f" || continue
  machos=$((machos + 1))
  info="$(codesign -dv --verbose=4 "$f" 2>&1)" || fail "unsigned Mach-O: $f"
  codesign --verify --strict "$f" 2>/dev/null || fail "invalid signature: $f"
  if [ "$SIGNED" = 1 ]; then
    echo "$info" | grep -q "TeamIdentifier=$TEAM" || fail "different Team ID: $f"
    echo "$info" | grep -Eq "flags=0x[0-9a-f]*\(.*runtime" || fail "no hardened runtime: $f"
    echo "$info" | grep -q "^Timestamp=" || fail "no secure timestamp: $f"
  fi
done < <(find "$APP" -type f -print0)
ok "$machos Mach-O files signed$([ "$SIGNED" = 1 ] && echo ", hardened runtime, timestamped, Team $TEAM")"

bad="$(find "$APP" -perm -g+w -o -perm -o+w | head -5)"
[ -z "$bad" ] || fail "group/other-writable files: $bad"
bad="$(find "$APP" \( -perm -4000 -o -perm -2000 -o -perm -1000 \) | head -5)"
[ -z "$bad" ] || fail "setuid/setgid/sticky files: $bad"
while IFS= read -r -d '' l; do
  t="$(cd "$(dirname "$l")" && realpath "$(readlink "$l")" 2>/dev/null || true)"
  case "$t" in "$(cd "$APP" && pwd -P)"/*) ;; *) fail "symlink escapes the bundle: $l" ;; esac
done < <(find "$APP" -type l -print0)
ok "permissions and symlinks"

# The worker, its lock and the runtime's recorded lock digest must be the
# ones in this checkout (the bundle may have been built elsewhere).
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cmp -s "$WORKER" "$REPO/workers/mlx/pegoles_mlx_worker.py" || fail "bundled worker differs from the checkout"
cmp -s "$APP/Contents/Resources/runtime/requirements.lock" "$REPO/workers/mlx/requirements.lock" \
  || fail "bundled requirements.lock differs from the checkout"
lock_sha="$(shasum -a 256 "$REPO/workers/mlx/requirements.lock" | cut -d' ' -f1)"
grep -q "\"requirements_lock_sha256\": \"$lock_sha\"" "$APP/Contents/Resources/runtime/runtime-manifest.json" \
  || fail "runtime manifest was not built from this checkout's lock"
ok "worker, lock and runtime manifest match the checkout"

# certifi's cacert.pem is the public Mozilla CA bundle (required by the
# HTTP stack mlx-vlm imports); any other PEM is refused.
forbidden="$(find "$APP" ! -path '*/site-packages/certifi/cacert.pem' \( -name '*.safetensors' -o -name '*.gguf' -o -name '*.pem' -o -name '*.p12' \
  -o -name '*.key' -o -name '.env' -o -name 'pyvenv.cfg' -o -name '*.img' -o -name '*.raw' \
  -o -name 'pip' -o -name 'ensurepip' \) | head -5)"
[ -z "$forbidden" ] || fail "forbidden content in bundle: $forbidden"
# Paths of this build machine must not leak into the bundle: the checkout
# anywhere, the builder's home in our own binaries. (Third-party wheel
# metadata legitimately mentions its own CI paths, e.g. /Users/runner/work.)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
if grep -rIlF --exclude='*.pyc' "$REPO_ROOT" "$APP" 2>/dev/null | grep -q .; then
  fail "checkout path in bundle: $(grep -rIlF "$REPO_ROOT" "$APP" | head -3)"
fi
for f in "$MAIN" "$HELPER"; do
  if strings -a "$f" | grep -qF -e "$HOME/" -e "$REPO_ROOT"; then
    fail "build-machine path embedded in $(basename "$f"): $(strings -a "$f" | grep -m3 -F -e "$HOME/" -e "$REPO_ROOT")"
  fi
done
ok "no weights, keys, venvs, installers or developer paths"

if [ "$DIST" = 1 ]; then
  spctl --assess --type execute -vv "$APP" 2>&1 | grep -q "source=Notarized Developer ID" \
    || fail "Gatekeeper: not accepted as a notarized Developer ID app"
  xcrun stapler validate "$APP" >/dev/null || fail "app has no stapled ticket"
  ok "Gatekeeper accepts (Notarized Developer ID), ticket stapled"
fi
echo "verified: $TARGET"

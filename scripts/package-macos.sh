#!/usr/bin/env bash
# Build, assemble and sign "Pegoles Agent.app" and its DMG from the
# current checkout. Nothing is published.
#
#   PEGOLES_SIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)" \
#     bash scripts/package-macos.sh
#
# Without PEGOLES_SIGN_IDENTITY the bundle is signed ad-hoc: usable on this
# Mac for testing, NOT distributable (no Team ID, no timestamp, and the
# bundled interpreter cannot use the hardened runtime because library
# validation needs a Team ID). Notarization is a separate step:
# scripts/release/notarize.sh.
#
# Bundle layout (everything the app runs is inside the signed bundle):
#   Contents/MacOS/pegoles-desktop        app (hardened runtime, no entitlements)
#   Contents/MacOS/pegoles-vm-host        VM helper (hardened runtime + virtualization)
#   Contents/Resources/runtime/python/    Pegoles Local interpreter + wheels
#   Contents/Resources/runtime/runtime-manifest.json
#   Contents/Resources/workers/mlx/pegoles_mlx_worker.py
#
# Signing is inside-out and explicit (no --deep): every Mach-O in the
# runtime, then the interpreter, then the helper, then the app.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
TAURI_DIR="$ROOT/apps/desktop/src-tauri"
HELPER_PLIST="$TAURI_DIR/entitlements/vm-host.plist"
IDENTITY="${PEGOLES_SIGN_IDENTITY:-}"
OUT="$ROOT/target/release-artifacts"

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || { echo "package on macOS / Apple silicon" >&2; exit 1; }

# --- versions must agree -------------------------------------------------
VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$TAURI_DIR/tauri.conf.json")"
CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$TAURI_DIR/Cargo.toml" | head -1)"
PKG_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT/apps/desktop/package.json")"
ROOT_PKG_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT/package.json")"
for v in "$CARGO_VERSION" "$PKG_VERSION" "$ROOT_PKG_VERSION"; do
  [ "$v" = "$VERSION" ] || { echo "version mismatch: tauri.conf.json=$VERSION vs $v" >&2; exit 1; }
done

if [ -n "$IDENTITY" ]; then
  security find-identity -v -p codesigning | grep -Fq "\"$IDENTITY\"" \
    || { echo "signing identity not found in the keychain: $IDENTITY" >&2; exit 1; }
  case "$IDENTITY" in
    "Developer ID Application:"*) ;;
    *) echo "distribution requires a 'Developer ID Application' identity" >&2; exit 1 ;;
  esac
else
  echo "WARNING: PEGOLES_SIGN_IDENTITY unset; ad-hoc signing. NOT FOR DISTRIBUTION." >&2
fi

# --- build ----------------------------------------------------------------
bash scripts/local-model/build-runtime.sh "$ROOT/target/pegoles-runtime"
# No developer paths in shipped binaries (panic locations, debug info):
# the checkout and the home directory are remapped. Rust applies the last
# matching mapping, so the more specific checkout comes after HOME.
# CARGO_ENCODED_RUSTFLAGS (0x1f-separated) because the checkout path may
# contain spaces.
CARGO_ENCODED_RUSTFLAGS="$(printf '%s\x1f%s' "--remap-path-prefix=$HOME=/home" "--remap-path-prefix=$ROOT=/pegoles")"
export CARGO_ENCODED_RUSTFLAGS
unset RUSTFLAGS
swift build -c release --package-path native/macos/pegoles-vm-host \
  -Xswiftc -file-prefix-map -Xswiftc "$ROOT=/pegoles" \
  -Xswiftc -file-prefix-map -Xswiftc "$HOME=/home"
pnpm --filter @pegoles/desktop tauri build --bundles app --no-sign

APP="$ROOT/target/release/bundle/macos/Pegoles Agent.app"
[ -d "$APP" ] || { echo "bundle not found at $APP" >&2; exit 1; }
MAIN="$APP/Contents/MacOS/pegoles-desktop"
[ -f "$MAIN" ] || { echo "main executable not found at $MAIN" >&2; exit 1; }

# --- assemble ---------------------------------------------------------------
install -m 0755 native/macos/pegoles-vm-host/.build/release/pegoles-vm-host "$APP/Contents/MacOS/pegoles-vm-host"
rm -rf "$APP/Contents/Resources/workers" "$APP/Contents/Resources/runtime"
mkdir -p "$APP/Contents/Resources/workers/mlx" "$APP/Contents/Resources/runtime"
install -m 0644 workers/mlx/pegoles_mlx_worker.py "$APP/Contents/Resources/workers/mlx/"
cp -R "$ROOT/target/pegoles-runtime/python" "$APP/Contents/Resources/runtime/python"
install -m 0644 "$ROOT/target/pegoles-runtime/runtime-manifest.json" workers/mlx/requirements.lock \
  "$APP/Contents/Resources/runtime/"
# Extended attributes (quarantine, Finder info) break code signatures.
xattr -cr "$APP"

# --- sign (inside-out, explicit) -----------------------------------------------
sign() { # sign <path> [runtime|noruntime] [entitlements.plist]
  local path="$1" mode="${2:-runtime}" ent="${3:-}"
  local args=(--force --sign "${IDENTITY:--}")
  if [ -n "$IDENTITY" ]; then
    args+=(--timestamp --options runtime)
  elif [ "$mode" = "runtime" ]; then
    args+=(--options runtime)
  fi
  [ -n "$ent" ] && args+=(--entitlements "$ent")
  codesign "${args[@]}" "$path"
}
is_macho() { file -b "$1" | grep -q '^Mach-O'; }

PY="$APP/Contents/Resources/runtime/python/bin/python3.12"
count=0
while IFS= read -r -d '' f; do
  [ "$f" = "$PY" ] && continue
  if is_macho "$f"; then sign "$f"; count=$((count + 1)); fi
done < <(find "$APP/Contents/Resources/runtime" -type f -print0)
echo "signed $count runtime libraries"
# Ad-hoc: library validation (hardened runtime) needs a Team ID, so the
# local interpreter is signed without it; Developer ID builds get it.
sign "$PY" noruntime
sign "$APP/Contents/MacOS/pegoles-vm-host" runtime "$HELPER_PLIST"
sign "$APP" runtime

# --- package ---------------------------------------------------------------------
rm -rf "$OUT"
mkdir -p "$OUT"
DMG="$OUT/Pegoles_${VERSION}_arm64.dmg"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/Pegoles Agent.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Pegoles Agent $VERSION" -srcfolder "$STAGE" -fs HFS+ \
  -format UDZO -imagekey zlib-level=9 -ov "$DMG"
if [ -n "$IDENTITY" ]; then
  codesign --force --timestamp --sign "$IDENTITY" "$DMG"
fi

bash scripts/release/verify-artifact.sh "$APP" ${IDENTITY:+--distribution}
echo "packaged: $APP"
echo "dmg:      $DMG"

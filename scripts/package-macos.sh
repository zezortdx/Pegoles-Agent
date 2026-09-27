#!/usr/bin/env bash
# Build, assemble and sign "Pegoles Agent.app" and its DMG from the
# current checkout. Nothing is published.
#
#   bash scripts/package-macos.sh [build|sign|all|app]    (default: all)
#
#   build  compile the runtime, helper and app and assemble an UNSIGNED
#          bundle at target/release/bundle/macos/ (no identity needed; the
#          release workflow runs this in a job without signing secrets)
#   sign   sign that bundle inside-out and build the DMG in
#          target/release-artifacts/ (runs only codesign, hdiutil and this
#          repository's scripts: no dependency code runs while the signing
#          identity is available)
#   app    build, then sign the bundle without making a DMG (the source
#          install path: scripts/install.sh)
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
STAGE_ARG="${1:-all}"
case "$STAGE_ARG" in build | sign | all | app) ;; *) echo "usage: package-macos.sh [build|sign|all|app]" >&2; exit 2 ;; esac
TAURI_DIR="$ROOT/apps/desktop/src-tauri"
HELPER_PLIST="$TAURI_DIR/entitlements/vm-host.plist"
IDENTITY="${PEGOLES_SIGN_IDENTITY:-}"
OUT="$ROOT/target/release-artifacts"
APP="$ROOT/target/release/bundle/macos/Pegoles Agent.app"

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || { echo "package on macOS / Apple silicon" >&2; exit 1; }

# --- versions must agree -------------------------------------------------
json_version() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$1"; }
VERSION="$(json_version "$TAURI_DIR/tauri.conf.json")"
CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$TAURI_DIR/Cargo.toml" | head -1)"
for v in "$CARGO_VERSION" "$(json_version "$ROOT/apps/desktop/package.json")" "$(json_version "$ROOT/package.json")"; do
  [ "$v" = "$VERSION" ] || { echo "version mismatch: tauri.conf.json=$VERSION vs $v" >&2; exit 1; }
done

build() {
  # No developer paths in shipped binaries (panic locations, debug info):
  # the checkout and the home directory are remapped. Rust applies the last
  # matching mapping, so the more specific checkout comes after HOME.
  # CARGO_ENCODED_RUSTFLAGS (0x1f-separated) because the checkout path may
  # contain spaces.
  CARGO_ENCODED_RUSTFLAGS="$(printf '%s\x1f%s' "--remap-path-prefix=$HOME=/home" "--remap-path-prefix=$ROOT=/pegoles")"
  export CARGO_ENCODED_RUSTFLAGS
  unset RUSTFLAGS
  bash scripts/local-model/build-runtime.sh "$ROOT/target/pegoles-runtime"
  swift build -c release --package-path native/macos/pegoles-vm-host \
    -Xswiftc -file-prefix-map -Xswiftc "$ROOT=/pegoles" \
    -Xswiftc -file-prefix-map -Xswiftc "$HOME=/home"
  pnpm --filter @pegoles/desktop tauri build --bundles app --no-sign

  [ -d "$APP" ] || { echo "bundle not found at $APP" >&2; exit 1; }
  [ -f "$APP/Contents/MacOS/pegoles-desktop" ] || { echo "main executable missing" >&2; exit 1; }
  install -m 0755 native/macos/pegoles-vm-host/.build/release/pegoles-vm-host "$APP/Contents/MacOS/pegoles-vm-host"
  rm -rf "$APP/Contents/Resources/workers" "$APP/Contents/Resources/runtime"
  mkdir -p "$APP/Contents/Resources/workers/mlx" "$APP/Contents/Resources/runtime"
  install -m 0644 workers/mlx/pegoles_mlx_worker.py "$APP/Contents/Resources/workers/mlx/"
  cp -R "$ROOT/target/pegoles-runtime/python" "$APP/Contents/Resources/runtime/python"
  install -m 0644 "$ROOT/target/pegoles-runtime/runtime-manifest.json" workers/mlx/requirements.lock \
    "$APP/Contents/Resources/runtime/"
  # Licenses of everything the bundle redistributes (Python runtime,
  # frontend packages, Rust crates) plus the app's own license.
  bash scripts/release/third-party-notices.sh "$ROOT/target/THIRD_PARTY_NOTICES.md"
  install -m 0644 "$ROOT/target/THIRD_PARTY_NOTICES.md" "$ROOT/LICENSE" "$APP/Contents/Resources/"
  echo "assembled (unsigned): $APP"
}

sign_bundle() {
  [ -d "$APP" ] || { echo "no assembled bundle at $APP (run the build stage)" >&2; exit 1; }
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
  # Extended attributes (quarantine, Finder info) break code signatures.
  xattr -cr "$APP"
  # Normalize modes: nothing setuid/setgid/sticky or writable by others.
  chmod -R u+rwX,go-w,-s,-t "$APP"

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
  # Mach-O by magic bytes (thin and universal, either byte order), not by
  # file(1)'s wording, which varies ("setuid Mach-O ...").
  is_macho() {
    case "$(head -c 4 "$1" 2>/dev/null | xxd -p)" in
      feedface | feedfacf | cefaedfe | cffaedfe | cafebabe | bebafeca) return 0 ;;
      *) return 1 ;;
    esac
  }

  local py="$APP/Contents/Resources/runtime/python/bin/python3.12" count=0
  while IFS= read -r -d '' f; do
    [ "$f" = "$py" ] && continue
    if is_macho "$f"; then sign "$f"; count=$((count + 1)); fi
  done < <(find "$APP/Contents/Resources/runtime" -type f -print0)
  echo "signed $count runtime libraries"
  # Ad-hoc: library validation (hardened runtime) needs a Team ID, so the
  # local interpreter is signed without it; Developer ID builds get it.
  sign "$py" noruntime
  sign "$APP/Contents/MacOS/pegoles-vm-host" runtime "$HELPER_PLIST"
  sign "$APP" runtime
  # --signed: Developer ID, Team ID, hardened runtime and timestamps.
  # Gatekeeper and stapling are checked after notarization (notarize.sh).
  bash scripts/release/verify-artifact.sh "$APP" ${IDENTITY:+--signed}
  echo "packaged: $APP"
}

make_dmg() {
  # Keep other release files already in $OUT (the build job's manifest and
  # SBOM); only the DMG is (re)made here.
  mkdir -p "$OUT"
  local dmg="$OUT/Pegoles_${VERSION}_arm64.dmg" stage
  rm -f "$dmg"
  stage="$(mktemp -d)"
  ditto "$APP" "$stage/Pegoles Agent.app"
  ln -s /Applications "$stage/Applications"
  hdiutil create -quiet -volname "Pegoles Agent $VERSION" -srcfolder "$stage" -fs HFS+ \
    -format UDZO -imagekey zlib-level=9 -ov "$dmg"
  rm -rf "$stage"
  if [ -n "$IDENTITY" ]; then
    codesign --force --timestamp --sign "$IDENTITY" "$dmg"
  fi
  echo "dmg:      $dmg"
}

case "$STAGE_ARG" in
  build) build ;;
  sign) sign_bundle; make_dmg ;;
  all) build; sign_bundle; make_dmg ;;
  app) build; sign_bundle ;;
esac

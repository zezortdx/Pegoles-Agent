#!/usr/bin/env bash
# Build "Pegoles Agent.app" with the VM helper inside it (Contents/MacOS,
# where the app resolves it in release builds) and sign both with the
# virtualization entitlement. Ad-hoc signing by default; set
# PEGOLES_SIGN_IDENTITY="Developer ID Application: …" for distribution
# (hardened runtime + timestamp; notarization is a separate step).
# Does not publish anything.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PLIST="$ROOT/apps/desktop/src-tauri/entitlements/macos.plist"
IDENTITY="${PEGOLES_SIGN_IDENTITY:--}"

swift build -c release --package-path native/macos/pegoles-vm-host
pnpm --filter @pegoles/desktop tauri build --bundles app

APP="$ROOT/target/release/bundle/macos/Pegoles Agent.app"
[ -d "$APP" ] || { echo "bundle not found at $APP"; exit 1; }
cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host "$APP/Contents/MacOS/pegoles-vm-host"

sign() {
  if [ "$IDENTITY" = "-" ]; then
    codesign --force --entitlements "$PLIST" -s - "$1"
  else
    codesign --force --options runtime --timestamp --entitlements "$PLIST" -s "$IDENTITY" "$1"
  fi
}
sign "$APP/Contents/MacOS/pegoles-vm-host"
sign "$APP"
codesign --verify --deep --strict "$APP"
codesign -d --entitlements - "$APP/Contents/MacOS/pegoles-vm-host" 2>/dev/null \
  | grep -q com.apple.security.virtualization
echo "packaged: $APP"

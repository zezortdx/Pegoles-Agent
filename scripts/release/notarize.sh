#!/usr/bin/env bash
# Notarize and staple a Developer ID build produced by scripts/package-macos.sh,
# then rebuild its DMG from the stapled app so both carry a ticket.
#
#   PEGOLES_SIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)" \
#   NOTARY_KEYCHAIN_PROFILE=<profile> bash scripts/release/notarize.sh      # local
#
#   PEGOLES_SIGN_IDENTITY=... NOTARY_KEY_ID=... NOTARY_ISSUER_ID=... \
#   NOTARY_KEY_P8_BASE64=... bash scripts/release/notarize.sh              # CI
#
# Credentials (never printed, never written outside a private temp dir):
#   NOTARY_KEYCHAIN_PROFILE   a profile stored with
#                             `xcrun notarytool store-credentials`, or
#   NOTARY_KEY_ID + NOTARY_ISSUER_ID + (NOTARY_KEY_PATH | NOTARY_KEY_P8_BASE64)
#                             an App Store Connect API key (.p8).
#
# Flow: zip the app (ditto --keepParent) -> notarytool submit --wait (on
# rejection the notarization log is printed) -> staple + validate the app
# -> rebuild the DMG exactly as package-macos.sh does -> sign it -> notarize
# the DMG -> staple it -> verify-artifact.sh --distribution on app and DMG.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
TAURI_DIR="$ROOT/apps/desktop/src-tauri"
APP="$ROOT/target/release/bundle/macos/Pegoles Agent.app"
OUT="$ROOT/target/release-artifacts"
IDENTITY="${PEGOLES_SIGN_IDENTITY:-}"
NOTARY_TIMEOUT="${NOTARY_TIMEOUT:-2h}"

fail() { echo "notarize: $*" >&2; exit 1; }

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || fail "run on macOS / Apple silicon"
case "$IDENTITY" in
  "Developer ID Application:"*) ;;
  *) fail "PEGOLES_SIGN_IDENTITY must be a 'Developer ID Application: ...' identity" ;;
esac
security find-identity -v -p codesigning | grep -Fq "\"$IDENTITY\"" \
  || fail "signing identity not found in the keychain: $IDENTITY"
[ -d "$APP" ] || fail "no app at $APP (run scripts/package-macos.sh first)"
VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$TAURI_DIR/tauri.conf.json")"
DMG="$OUT/Pegoles_${VERSION}_arm64.dmg"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

AUTH=()
if [ -n "${NOTARY_KEYCHAIN_PROFILE:-}" ]; then
  AUTH=(--keychain-profile "$NOTARY_KEYCHAIN_PROFILE")
elif [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER_ID:-}" ]; then
  KEY="${NOTARY_KEY_PATH:-}"
  if [ -z "$KEY" ]; then
    [ -n "${NOTARY_KEY_P8_BASE64:-}" ] || fail "set NOTARY_KEY_PATH or NOTARY_KEY_P8_BASE64"
    KEY="$WORK/AuthKey_${NOTARY_KEY_ID}.p8"
    (umask 077 && printf '%s' "$NOTARY_KEY_P8_BASE64" | base64 --decode > "$KEY")
  fi
  [ -s "$KEY" ] || fail "API key file is empty or missing"
  AUTH=(--key "$KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
else
  fail "no notarization credentials (NOTARY_KEYCHAIN_PROFILE, or NOTARY_KEY_ID + NOTARY_ISSUER_ID + key)"
fi

# The app must already be a valid Developer ID + hardened-runtime build;
# notarization would reject anything else after a long round trip.
codesign --verify --strict --verbose=2 "$APP" >/dev/null 2>&1 || fail "app signature is invalid"
codesign -dv --verbose=4 "$APP" 2>&1 | grep -q "^Authority=Developer ID Application:" \
  || fail "app is not signed with a Developer ID Application certificate"

json_field() { # json_field <file> <key>
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2], ""))' "$1" "$2"
}

notarize() { # notarize <file> <label>
  local file="$1" label="$2" result="$WORK/$2.json" id status
  echo "==> notarytool submit ($label)"
  if ! xcrun notarytool submit "$file" "${AUTH[@]}" --wait --timeout "$NOTARY_TIMEOUT" \
    --output-format json > "$result"; then
    cat "$result" >&2 || true
  fi
  id="$(json_field "$result" id 2>/dev/null || true)"
  status="$(json_field "$result" status 2>/dev/null || true)"
  echo "    submission ${id:-<none>}: ${status:-<no status>}"
  if [ "$status" != "Accepted" ]; then
    if [ -n "$id" ]; then
      echo "==> notarization log ($label)" >&2
      xcrun notarytool log "$id" "${AUTH[@]}" >&2 || true
    fi
    fail "$label was not accepted by the notary service (${status:-no status})"
  fi
}

# --- app -----------------------------------------------------------------------
ditto -c -k --keepParent "$APP" "$WORK/app.zip"
notarize "$WORK/app.zip" app
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"

# --- DMG ---------------------------------------------------------------------------
# Mirrors the "package" step of scripts/package-macos.sh exactly (same
# layout, volume name, filesystem and compression); keep the two in sync.
mkdir -p "$OUT"
STAGE="$WORK/dmg"
mkdir "$STAGE"
ditto "$APP" "$STAGE/Pegoles Agent.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Pegoles Agent $VERSION" -srcfolder "$STAGE" -fs HFS+ \
  -format UDZO -imagekey zlib-level=9 -ov "$DMG"
codesign --force --timestamp --sign "$IDENTITY" "$DMG"
notarize "$DMG" dmg
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

# --- verify ------------------------------------------------------------------------
bash scripts/release/verify-artifact.sh "$APP" --distribution
bash scripts/release/verify-artifact.sh "$DMG" --distribution
echo "notarized: $APP"
echo "dmg:       $DMG"

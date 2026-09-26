#!/usr/bin/env bash
# Re-sign the locally built VM helper ad-hoc with the virtualization
# entitlement. The helper is the only process that uses
# Virtualization.framework; without the entitlement VZVirtualMachine
# refuses to start (backend error `not_entitled`). The app binary itself
# needs no entitlement. Development only: releases are signed by
# scripts/package-macos.sh with a Developer ID.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLIST="$ROOT/apps/desktop/src-tauri/entitlements/vm-host.plist"
HELPER="$ROOT/native/macos/pegoles-vm-host/.build/release/pegoles-vm-host"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "codesign-dev.sh is macOS-only" >&2
  exit 1
fi
if [[ ! -f "$HELPER" ]]; then
  echo "helper not found at $HELPER (swift build -c release --package-path native/macos/pegoles-vm-host)" >&2
  exit 1
fi
codesign --entitlements "$PLIST" -f -s - "$HELPER"
codesign -d --entitlements - "$HELPER" 2>/dev/null | grep -q "com.apple.security.virtualization"
echo "signed $HELPER with the virtualization entitlement"

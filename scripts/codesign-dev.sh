#!/usr/bin/env bash
# Re-sign the dev binary ad-hoc with the virtualization entitlement.
# `tauri dev` runs a cargo-built binary without entitlements, and
# Virtualization.framework refuses to start VMs in that case
# (backend error `not_entitled`). Release bundles get the entitlement
# from tauri.conf.json -> bundle.macOS.entitlements automatically.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/debug/pegoles-desktop"
PLIST="$ROOT/apps/desktop/src-tauri/entitlements/macos.plist"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "codesign-dev.sh is macOS-only" >&2
  exit 1
fi
if [[ ! -f "$BIN" ]]; then
  echo "dev binary not found at $BIN (run cargo build first)" >&2
  exit 1
fi
codesign --entitlements "$PLIST" -f -s - "$BIN"
echo "signed $BIN with virtualization entitlement"
codesign -d --entitlements - "$BIN" | grep -q "com.apple.security.virtualization" \
  && echo "entitlement present"

# The VM host helper instantiates VZVirtualMachine itself, so it needs the
# same entitlement in its own signature.
HELPER="$ROOT/native/macos/pegoles-vm-host/.build/release/pegoles-vm-host"
if [[ -f "$HELPER" ]]; then
  codesign --entitlements "$PLIST" -f -s - "$HELPER"
  echo "signed $HELPER with virtualization entitlement"
fi

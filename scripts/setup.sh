#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> rust toolchain"
rustc --version
cargo --version

echo "==> node/pnpm"
node --version
pnpm --version

echo "==> install workspaces"
pnpm install

echo "done. Run 'pnpm --filter @pegoles/desktop tauri dev' to open the app."

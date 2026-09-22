#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "==> cargo test"
cargo test

if [ -f "apps/desktop/package.json" ]; then
  echo "==> frontend lint/typecheck/build"
  pnpm --filter @pegoles/desktop lint
  pnpm --filter @pegoles/desktop typecheck
  pnpm --filter @pegoles/desktop build
fi

echo "==> all checks passed"

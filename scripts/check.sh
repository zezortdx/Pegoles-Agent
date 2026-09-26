#!/usr/bin/env bash
# All local gates. Hardware E2E is separate (needs a Mac + sealed image):
# crates/pegoles-agent/examples/agent_e2e.rs
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "==> cargo test"
cargo test

if [ "$(uname)" = "Darwin" ]; then
  echo "==> swift build (VM helper)"
  swift build -c release --package-path native/macos/pegoles-vm-host
fi

if [ -f "apps/desktop/package.json" ]; then
  echo "==> frontend lint/typecheck/test/build"
  pnpm --filter @pegoles/desktop lint
  pnpm --filter @pegoles/desktop typecheck
  pnpm --filter @pegoles/desktop test
  pnpm --filter @pegoles/desktop build
fi

echo "==> all checks passed"

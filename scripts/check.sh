#!/usr/bin/env bash
# All local gates. Hardware E2E is separate (needs a Mac + sealed image):
# crates/pegoles-agent/examples/agent_e2e.rs
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The frontend comes first: pegoles-desktop embeds apps/desktop/dist at
# compile time, so cargo cannot build it on a fresh clone before this.
if [ -f "apps/desktop/package.json" ]; then
  echo "==> frontend lint/typecheck/test/build"
  pnpm --filter @pegoles/desktop lint
  pnpm --filter @pegoles/desktop typecheck
  pnpm --filter @pegoles/desktop test
  pnpm --filter @pegoles/desktop build
  echo "==> production bundle (no dev labs, debug commands or developer instructions)"
  # Checks the bundle just built; fails if it is missing.
  PEGOLES_REQUIRE_BUNDLE=1 pnpm --filter @pegoles/desktop exec vitest run src/dev/prodBundle.test.ts
fi

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy"
cargo clippy --locked --all-targets -- -D warnings

echo "==> cargo test"
cargo test --locked

if [ "$(uname)" = "Darwin" ]; then
  echo "==> swift build (VM helper)"
  swift build -c release --package-path native/macos/pegoles-vm-host
fi

echo "==> all checks passed"

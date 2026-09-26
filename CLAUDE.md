# Pegoles — notes for coding sessions

Secure computer-use agent. Model proposes typed actions → deterministic
policy → Core executor → isolated macOS VM (no network). Current state,
platform truth and next steps: `docs/PROJECT_STATE.md`. Security model:
`docs/SECURITY.md`. Read those before large changes.

## Gates

- `bash scripts/check.sh` — fmt, clippy `-D warnings`, cargo test, Swift
  build, frontend lint/typecheck/vitest/build. Keep it green.
- Hardware E2E (Apple Silicon, sealed image `pegoles-base-0.3`):
  `cargo build --release -p pegoles-agent --example agent_e2e`, copy
  `native/macos/pegoles-vm-host/.build/release/pegoles-vm-host` next to
  the example binary, run it. Release builds ignore dev overrides.
- The Swift helper must be re-signed after every rebuild:
  `codesign --entitlements apps/desktop/src-tauri/entitlements/macos.plist -f -s - <helper>`.

## Invariants (do not break)

- No host shell/file/process/URL action may exist in
  `pegoles-protocol::ComputerAction`; `pegoles-policy::evaluate` stays an
  exhaustive match; every action (incl. waits) goes through Core.
- Guest data is hostile: bound every field, never panic on it, never
  hold the app lock while waiting on the guest longer than needed.
- The model key lives in the Keychain; never log it, return it to the
  webview, or put it in model context.
- Only a sealed Pegoles image boots in normal flows (fail closed).
- The frontend never renders model/guest text as HTML.

## Guest image loop (fast)

`scripts/build-guest-image/build-runtime.sh <out>` (Docker) →
`patch-image.sh <provisioned.img> <runtime> <out.img>` (debugfs, no boot)
→ `PEGOLES_DEBS_DIR=<debs> cargo run -p pegoles-computer --example
seal_image -- <out.img>`. Unit files live in
`scripts/build-guest-image/seed/units/` and feed both the full cloud-init
build and the patch path. The repo path contains a space: quote paths.

## Environment gotchas

- Port 1420 on the dev machine belongs to another project; run Vite on
  1430 (`npx vite --port 1430 --strictPort` in apps/desktop) and override
  `devUrl` for `tauri dev`.
- Disk space is tight; each VM computer is an APFS clone of the image but
  grows as the guest writes. Destroy test computers.
- The UI can be previewed without a backend at `#/dev/shell/<scenario>`.

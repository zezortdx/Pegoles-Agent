# Pegoles — notes for coding sessions

Secure computer-use agent. Model proposes typed actions → deterministic
policy → Core executor → isolated macOS VM (no network). Default planner
is Pegoles Local: a small VLM on the host (MLX worker), no API key;
Anthropic is optional. Current state, platform truth and next steps:
`docs/PROJECT_STATE.md`. Security model: `docs/SECURITY.md`. Planners,
providers, worker, model store: `docs/MODEL_ARCHITECTURE.md`. Read
those before large changes.

## Gates

- `bash scripts/check.sh` — fmt, clippy `-D warnings`, cargo test, Swift
  build, frontend lint/typecheck/vitest/build. Keep it green.
- Hardware E2E (Apple Silicon, sealed image `pegoles-base-0.3`):
  `cargo build --release -p pegoles-agent --example agent_e2e`, copy
  `native/macos/pegoles-vm-host/.build/release/pegoles-vm-host` next to
  the example binary, run it. Release builds ignore dev overrides.
- The Swift helper must be re-signed after every rebuild:
  `codesign --entitlements apps/desktop/src-tauri/entitlements/macos.plist -f -s - <helper>`.

## Local models

- Runtime: `bash scripts/local-model/setup-runtime.sh` (hash-locked
  venv at `<data>/runtime/mlx-venv`). Models: `cargo run -p
  pegoles-inference --example models -- list|install <id>|verify <id>`
  (weights live in `<data>/models`, never in Git; catalog pinned in
  `crates/pegoles-inference/catalog/models.json`).
- Benchmark on the real VM: `local_bench` (see
  `benchmarks/local-models/README.md`); keyless product-path E2E:
  `cargo build --release -p pegoles-desktop --example local_e2e`, copy the
  signed helper next to it, run.
- The worker must stay sandboxed (`sandbox-exec`, fail closed) with a
  cleared environment; model output is only ever parsed by
  `pegoles-agent/src/local/parse.rs` into typed actions.

## Invariants (do not break)

- No host shell/file/process/URL action may exist in
  `pegoles-protocol::ComputerAction`; `pegoles-policy::evaluate` stays an
  exhaustive match; every action (incl. waits) goes through Core.
- Guest data is hostile: bound every field, never panic on it, never
  hold the app lock while waiting on the guest longer than needed.
- The model key lives in the Keychain; never log it, return it to the
  webview, put it in model context, or pass it to the local worker.
- Every provider (local or cloud) is an untrusted planner behind the
  same `Planner` trait and the same policy path.
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

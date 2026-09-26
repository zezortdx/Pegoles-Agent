# Platform Matrix (honest status, 2026-09-26)

Tiers: **VERIFIED** = exercised on real hardware in this repo's E2E;
**IMPLEMENTED** = code + unit tests, not run on real hardware;
**NOT IMPLEMENTED** = absent.

## macOS Apple Silicon (arm64) — reference platform

| Area | Status |
|---|---|
| VM lifecycle (create/start/pause/resume/stop/reset/destroy) | VERIFIED (reset restores the sealed disk: unit-tested; stop/destroy/second boot: E2E) |
| Guest control plane (authenticated vsock, handshake, heartbeat, reconnect) | VERIFIED (`agent_e2e`: runtime killed → recovered ≈ 3 s) |
| Computer use (observe, click, type incl. dead-key chars, keys, wait) | VERIFIED (`agent_e2e`, pixel-checked) |
| Drag, scroll, double-click, key chords | IMPLEMENTED end to end; exercised by the older fixture harness on image v0.2, not by `agent_e2e` |
| Agent orchestrator with a scripted planner | VERIFIED (`agent_e2e`) |
| Agent orchestrator with Claude (Anthropic API) | IMPLEMENTED (request/response/translation unit-tested); NOT run against the live API in this environment (no key) |
| Desktop UI wiring (tasks, settings, computer controls) | IMPLEMENTED (vitest); live app not visually verified by the agent |
| Native VM display embed (human takes control) | NOT IMPLEMENTED (`pegoles-macos-embed` is a stub); the UI shows captured frames instead |
| Image distribution (download a sealed Pegoles image) | NOT IMPLEMENTED — images are built locally (`scripts/build-guest-image`) |

## Windows 11 (Hyper-V / HCS)

| Area | Status |
|---|---|
| HCS backend, Hyper-V socket transport, setup tool | Code exists behind `cfg(windows)`. Never booted. Two compile errors and two Windows-API bugs were fixed by inspection on 2026-09-26; the crate was **not compiled** for Windows in this environment. |
| Guest image for Windows | amd64 VHDX is unprovisioned; the runtime is built for arm64 only |

**Not supported.** The UI must not claim Windows support.

## Linux host

NOT IMPLEMENTED (Mock backend only, for tests). Shared crates compile and
test on Linux CI.

## Guest images

| Image | Status |
|---|---|
| `pegoles-base-0.3` (Debian 13 arm64, weston + foot, runtime 0.2.0, authenticated channel, workspace terminal) | VERIFIED; product default |
| `pegoles-base-0.2` | Superseded (runtime cannot authenticate; the host rejects it) |
| `pegoles-base-0.1` | Superseded (headless, no input/frame) |

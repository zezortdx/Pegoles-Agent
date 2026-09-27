# Platform Matrix (honest status, 2026-09-27)

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

## Windows 11 x64 (Host Compute System) — in development

Branch `phase/windows-0.2`; design and reasoning in
`docs/WINDOWS_ARCHITECTURE.md`. **No Windows PC has run Pegoles yet.**
Extra tier: **CI** = exercised on GitHub's Windows Server 2025 runners
(Azure VMs with nested virtualization, as an administrator), which is
real Windows but not a consumer PC.

| Area | Status |
|---|---|
| Broker service (`PegolesVmBroker`) + typed pipe protocol | IMPLEMENTED; CI: linted and unit-tested on Windows |
| Unprivileged VM helper (`pegoles-vm-host.exe`, same JSONL as macOS) | IMPLEMENTED; CI: linted and unit-tested on Windows |
| HCS VM boot (UEFI from VHDX, no network) | CI: a firmware-only VM and the real x64 guest both boot through the broker and the helper (Windows Server 2025, nested); the COM1 log came back empty (fix pending in CI) |
| x64 guest image (Debian 13 amd64, runtime in listen mode, Hyper-V drivers) | CI: built and provisioned under KVM; boots under QEMU/UEFI with its services started; **not published** — setup on Windows reports "not available yet" |
| Guest channel (AF_HYPERV → guest listener on vsock 850) | CI: the helper reaches the booted guest runtime (transport level; the full handshake, frames and input on Windows are not exercised yet) |
| Pegoles Local (llama.cpp, GGUF Q8, Vulkan/CPU) in AppContainer + job | Worker VERIFIED on macOS (Metal) against the real VM (`local_bench`); Windows sandbox IMPLEMENTED; CI start test pending |
| Onboarding (system check, turning on virtualization, restart and resume) | IMPLEMENTED (UI tested with fixtures; Windows facts from real APIs, cross-compiled); never run on Windows |
| Webview network containment (WebView2 switches) | CI: egress probe on Windows — 0 TCP / 0 UDP contained (33 / 353 without) |
| Installer (NSIS, per machine, `Pegoles-Setup-x64.exe`) | Scripted; CI silent install/uninstall pending first run; unsigned |
| Cloud planner | NOT AVAILABLE on Windows (fails closed: no native confirmation window yet) |
| Windows on ARM | NOT IMPLEMENTED |

**Not released.** The UI must not claim Windows support until the gates
in `docs/RELEASE_GATES.md` (Windows section) pass on a real PC.

## Linux host

NOT IMPLEMENTED (Mock backend only, for tests). Shared crates compile and
test on Linux CI.

## Guest images

| Image | Status |
|---|---|
| `pegoles-base-0.3` (Debian 13 arm64, weston + foot, runtime 0.2.0, authenticated channel, workspace terminal) | VERIFIED; product default |
| `pegoles-base-0.2` | Superseded (runtime cannot authenticate; the host rejects it) |
| `pegoles-base-0.1` | Superseded (headless, no input/frame) |

# Pegoles Agent

**Local brain. Isolated computer. Anywhere.**

Pegoles is a local-first autonomous agent that runs AI models on your hardware and gives them an isolated computer to work inside.

> **AI gets its own computer. Not your computer.**

- **Local models** — inference on your hardware (roadmap)
- **Isolated computer** — agent acts inside a VM, never directly on your host
- **Mobile control** — phone as encrypted thin client (roadmap)
- **Open source** — MIT
- **Security-first** — deterministic policy engine, structured actions, VM boundary ([docs/SECURITY.md](docs/SECURITY.md))

## What works today (Phase 3.6)

- [x] Everything from 3.5, plus:
- [x] **Real Windows backend structure**: `WindowsHcsBackend` on the shared engine (create/start/pause/resume/stop/state/destroy, VHDX-only, no Mock fallback) — compiles everywhere, runs on Windows hardware (not available in this phase)
- [x] **HCS bindings**: ComputeCore.dll FFI declarations (verified signatures), RAII handles, operation runner, Secure Boot (Linux CA template) VM config builder
- [x] **Real Windows probe**: registry/SCM/security/LibLoader detection, honest Unknowns, 10 support states
- [x] **`pegoles-vm-host.exe`**: HCS session, HyperVSocketTransport (AF_HYPERV), same JSONL protocol
- [x] **`pegoles-windows-setup`**: check/explain/register (narrow, verify-after-write)
- [x] **VHDX artifact**: real bytes + real hash + kernel gate (`universal_ready: true` on amd64 too), manifest with per-artifact records
- [x] **Windows e2e test** (`PEGOLES_REAL_WINDOWS_VM_TEST=1`, incl. second-start instance check) — runs on hardware only
- [x] **Performance foundation**: ResourceGovernor + profiles, balloon device + policy, log rotation, `bench.sh`, PERFORMANCE.md baseline, reduced-motion, model-router contract
- [x] Minimal Windows UI (Hyper-V line + setup steps), `suggested_config` API

## Honesty tiers used in this repo

- IMPLEMENTED: written, doc-sourced, unit-tested where hardware-independent.
- CI VERIFIED: green on Windows/Linux/macOS runners (no real VMs there).
- REAL HARDWARE VERIFIED: macOS Apple Silicon rows only (VM + guest e2e re-run this phase).

## Roadmap (not built yet)

Windows hardware validation · Local model router · Agent Engine · VM streaming · computer-use · browser automation · Weston/Chromium guest UI · remote access · mobile app · Host Bridge.

## Run

```bash
bash scripts/setup.sh
cargo test
pnpm --filter @pegoles/desktop tauri dev
```

Real VM on macOS arm64:

```bash
cd native/macos/pegoles-vm-host && swift build -c release && cd -
bash scripts/codesign-dev.sh   # ad-hoc entitlement for `tauri dev`
pnpm --filter @pegoles/desktop tauri dev  # Prepare Computer -> Create -> Start
PEGOLES_REAL_VM_TEST=1 cargo test -p pegoles-computer real_vm  # hardware smoke
```

Full gates: `bash scripts/check.sh` (fmt + clippy + cargo test + frontend lint/typecheck/build).

Requires: Rust stable, Node ≥ 20, pnpm 9. macOS for the desktop app.

## Layout

```text
apps/desktop        Tauri 2 + React + TypeScript
crates/             pegoles-protocol, pegoles-policy, pegoles-computer, pegoles-core,
                    pegoles-guest-proto
guest/runtime       pegoles-guest-runtime (Linux ARM64) + systemd unit
native/macos        Swift pegoles-vm-host (Virtualization.framework)
native/windows      Rust pegoles-vm-host (HCS session + AF_HYPERV) + pegoles-windows-setup
packages/ui         Shared design tokens
scripts/            check.sh, bench.sh, build-guest-image/
docs/               ARCHITECTURE, SECURITY, PEGOLES_COMPUTER, TAURI_IPC,
                    VM_HOST_PROTOCOL, GUEST_PROTOCOL, GUEST_RUNTIME,
                    DEBIAN_IMAGE, WINDOWS_IMAGE, PORTABILITY_AUDIT,
                    WINDOWS_BACKEND, WINDOWS_SETUP, PLATFORM_MATRIX,
                    PERFORMANCE
```

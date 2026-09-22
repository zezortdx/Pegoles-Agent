# Platform Matrix (honest status, Phase 3.6)

## macOS Apple Silicon (arm64)

| Area | Status |
|---|---|
| VM lifecycle (create/validate/start/pause/resume/stop) | VERIFIED on hardware |
| Guest protocol v1 (vsock handshake/heartbeat/reconnect) | VERIFIED on hardware (Phase 3) |
| Base image (Debian 13 arm64 raw, verified, derived v0.1) | VERIFIED |
| Kernel transports (virtio + hyperv vsock modules) | VERIFIED (`universal_ready: true`) |
| UI (status, guest runtime, timeline) | IMPLEMENTED (visual run = local `tauri dev`) |
| Graphical display / computer-use | NOT IN SCOPE |

## Windows 11 Pro x86_64 (Hyper-V / HCS target)

| Area | Status |
|---|---|
| Portable architecture (enums, paths, factory, capabilities) | IMPLEMENTED, tested |
| `WindowsHcsBackend` skeleton | COMPILES everywhere; lifecycle = explicit `BackendFeatureNotImplemented` |
| Host capability probe + support states | IMPLEMENTED, live OS APIs (classify tested; behavior unverified without hardware) |
| Hyper-V socket design + service GUID derivation | SPECIFIED + tested (`00000FD2-…` for 4050) |
| `pegoles-vm-host.exe` (HCS session, config builder, error mapping) | IMPLEMENTED, compiles; lifecycle calls are real FFI, unverified on hardware |
| `HyperVSocketTransport` (AF_HYPERV, framing, reconnect) | IMPLEMENTED + unit-tested via byte streams; socket syscalls unverified on hardware |
| `pegoles-windows-setup` (check/explain/register) | IMPLEMENTED; register path unverified on hardware |
| Privileged setup model | SPECIFIED (`WINDOWS_SETUP.md`); runtime verify-only |
| HCS lifecycle on real VM / hardware validation | NOT VERIFIED (no Windows hardware in this phase) |
| Windows Debian VHDX artifact | BUILT (unprovisioned, real hash, kernel-gated) — see WINDOWS_IMAGE.md |
| Windows CI compile gate | IMPLEMENTED (this repo) |

## What IMPLEMENTED vs VERIFIED means here

- IMPLEMENTED: code written, reviewed against official docs, unit-tested
  where hardware-independent.
- CI VERIFIED: compiles + tests pass on Windows/Linux/macOS runners.
- REAL HARDWARE VERIFIED: only macOS rows below. NOTHING in the Windows
  rows claims hardware verification — see DoD honesty rule.

## Windows Home

NOT SUPPORTED by the HCS backend. Future `WindowsWhpBackend` is
FUTURE / RESEARCH only. No hacks, no unsupported install scripts.

## Windows ARM64

NOT IMPLEMENTED. Possible by design: `GuestArchitecture::Arm64` is
platform-independent (not a Mac synonym); no code assumes Arm64 ⇒ Mac.

## Linux (host)

NOT IMPLEMENTED (future KVM backend). Shared crates compile + test on
Linux CI; Mock backend works; no hypervisor code exists.

## Guest images

| Logical image | Artifact | Status |
|---|---|---|
| Pegoles Base Image v0.1 | macOS/arm64 `.raw` (Debian 13 + runtime 0.1 + protocol 1) | BUILT, verified, booted |
| Pegoles Base Image v0.1 | Windows/amd64 `.vhdx` (Debian 13 generic, UNPROVISIONED) | BUILT (real hash, kernel-gated); runtime provisioning happens on first Windows boot |

## Performance baseline (this phase, macOS)

See `docs/PERFORMANCE.md` + `scripts/bench.sh`: GuestReady ~6 s,
ping 0 ms, helper ~9 MiB RSS / 0.0% idle CPU, sparse RAW + dynamic VHDX.

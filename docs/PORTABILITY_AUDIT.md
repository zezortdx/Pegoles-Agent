# Portability Audit (Phase 3.5)

Every occurrence of platform-specific concepts in shared code, classified.
Method: `grep` over `crates/*/src`, `guest/runtime/src`,
`apps/desktop/src-tauri/src` for
`Virtualization.framework|VZVirtio|VZVirtualMachine|macOS|arm64|
RAW|Swift|HCS|Hyper-V|Win32|Application Support` (+ frontend `platform`
usage). Audit date: Phase 3.5; re-run the greps before Phase 4.

## Verdict summary

Shared crates (`pegoles-protocol`, `pegoles-guest-proto`, `pegoles-policy`)
are clean — enforced by `portability_tests::shared_crates_stay_platform_free`.
All hypervisor specifics live in adapters (`macos.rs`, `windows.rs`,
`native/*`) or the explicit `platform` vocabulary module. No redesign of
working code was needed; the leaks found were vocabulary-level and are
fixed by the platform module + factory.

## PLATFORM-SPECIFIC — correct, untouched

| Location | Concept | Why correct |
|---|---|---|
| `computer/src/macos.rs` | Virtualization.framework, `native/macos` helper path, EFI vars, `VZVirtualMachine.State` mapping | The macOS adapter; that IS its job |
| `native/macos/*` (Swift) | Vz APIs, main-queue routing, EFI variable store | Platform boundary by design |
| `native/windows/*` | HCS direction docs, skeleton binary | Platform boundary by design |
| `windows.rs` | HCS, Hyper-V GUIDs, registry paths, `cfg(windows)` probe | The Windows adapter |
| `platform.rs`, `config.rs` gates | `cfg(target_os/arch)`, `is_real_backend_supported()` | Centralized capability detection |
| `main.rs` `windows_subsystem` | Windows console suppression | Correct per-platform cfg |
| Smoke-test gates (`PEGOLES_REAL_VM_TEST`, hardware asserts) | macOS/arm64 requirements | Tests, explicit and opt-in |
| CSS `-apple-system` font stack | Fallback font list | Progressive enhancement, harmless |
| `is_host_path()` `C:\` deny entries | Windows path SHAPES in a denylist | Must RECOGNIZE host paths to DENY them; not a dependency (see below) |

## PLATFORM LEAK — fixed in 3.5

| # | Leak | Fix |
|---|---|---|
| 1 | `config::pegoles_data_dir()` hardcoded `~/Library/Application Support` with inline `#[cfg]`, `.pegoles` fallback elsewhere | `platform::PegolesPaths` (`for_current_host/for_platform`); macOS / `%LOCALAPPDATA%` / XDG; `config` delegates. Tested per platform |
| 2 | `ComputerConfig` had no arch/disk-format concepts; defaults not arch-aware | `GuestArchitecture::{Arm64,X86_64}`, `DiskFormat::{Raw,Vhdx}`, `BackendCapabilities`; per-backend `capabilities()` |
| 3 | `commands.rs` hardcoded `spec_os/spec_arch` strings | Values now come from backend caps + image model (status carries real spec) |
| 4 | Core imported `MacOSVirtualizationBackend` directly | `platform::create_backend()` factory; Core never names adapters. `BackendKind` moved to platform (`Mock/MacOSVirtualization/WindowsHcs`) |
| 5 | vsock handling implicit inside `macos.rs` + Swift, no named boundary | `GuestTransport` trait (`send_frame/poll_events/close/is_connected`) + `TransportEvent`; macOS implements it; `FakeGuestTransport` proves session-above-transport |
| 6 | Guest runtime hardcoded `svm_cid = 2` inline | `connector.rs::resolve_host_endpoint()` isolates endpoint discovery; same binary works on virtio/Hyper-V/vhost |
| 7 | `ImageSpec` stringly-typed (`arch: "arm64"`), single artifact, no host mapping | `ImageFamily` (logical) vs `PlatformArtifact` (host+arch+format+file+hash); `disk_format_for()`; per-artifact manifest records |
| 8 | No host capability surface; frontend would need user-agent sniffing | `host_capabilities()` + `get_host_capabilities` command; UI renders Rust-provided data only |
| 9 | `ComputerId` doubled as the backend VM handle | `ComputerInstance` (ephemeral, minted per start, cleared on stop); computer identity stable across restarts |
| 10 | Shared-crate docs named `Virtualization.framework`/`Swift` | Reworded to generic terms; tripwire test locks it |

## Deliberately NOT changed

- `vmhost_proto.rs` mentions Swift/VZ in comments: it IS the macOS wire
  protocol; the Windows host will speak the same command SET (documented
  in `WINDOWS_BACKEND.md`), not the same bytes.
- `serial_log_path` on the trait: serial consoles exist on Hyper-V too
  (COM ports); the concept is generic, the attachment is per-adapter.
- `GuestMessage` test fixtures saying `"arch":"aarch64"`: values, not
  coupling.
- Policy `C:\`/`~` denylist entries: recognizing attacker-controlled
  shapes to reject them is the opposite of depending on them.

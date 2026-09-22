# native/windows — HCS boundary (Phase 3.5: skeleton)

Future home of the Windows VM host companion (`pegoles-vm-host.exe`):
Rust first, using ComputeCore.dll / Host Compute System APIs for
lifecycle, Winsock AF_HYPERV for the guest control plane, and Virtual
Disk APIs for VHDX. See `docs/WINDOWS_BACKEND.md`.

## Rules (mirroring native/macos)

1. The Windows helper speaks the SAME JSONL command set as macOS
   (`version/validate/create/start/pause/resume/stop/state/destroy` +
   `guest_send/guest_status/guest_disconnect`). No `windowsStart` fork.
2. No PowerShell as a runtime backend. No Hyper-V Manager GUI. No WMI
   scripts. C++ only if HCS interop demands it.
3. Rust owns lifecycle/policy/events; the helper owns HCS handles,
   socket I/O, and transport errors only.
4. Registry of the Hyper-V socket service is a one-time privileged
   installer step (`docs/WINDOWS_SETUP.md`); runtime only verifies.

## Layout

```text
native/windows/
  README.md              # this file
  pegoles-vm-host/       # Rust skeleton (compiles everywhere today)
    Cargo.toml
    src/main.rs          # --version / --probe / JSONL not-implemented loop
```

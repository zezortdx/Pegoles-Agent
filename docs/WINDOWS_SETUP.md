# Windows Setup (one-time privileged steps)

> **Superseded (2026-09-27).** This describes the Phase 3.6 setup tool (privileged steps the person ran by hand). The current design is in `docs/WINDOWS_ARCHITECTURE.md` (Host Compute System with only the Virtual Machine Platform, a small broker service, the guest runtime listening on vsock 850, an image provisioned at build time by `scripts/build-guest-image/build-x64.sh`). Kept for history.

Pegoles on Windows separates **one-time privileged setup** from the
**unprivileged runtime**. Nothing here runs silently during normal use:
each step is explicit, checkable via the capability probe
(`WindowsHostCapabilities`), and surfaced in the UI as a concrete state
(`SetupRequired`, `HvSocketRegistrationMissing`, …) — never a bare
"Windows unsupported".

## Requirements

- Windows 11 Pro / Enterprise / Education, x86_64 (Home is out of scope
  for the HCS backend; see WHP note in `WINDOWS_BACKEND.md`).
- Hardware virtualization enabled in firmware (Intel VT-x / AMD-V).
- Hyper-V feature enabled + one reboot.
- User in Hyper-V Administrators (checked, never granted silently).

## Steps

1. **Firmware**: enable Intel VT-x / AMD-V.
   Verify: Task Manager → Performance → Virtualization: Enabled.
2. **Enable Hyper-V**: Settings → Apps → Optional features →
   More Windows features → Hyper-V (Platform + Management Tools) →
   **reboot**.
3. **Permissions**: add the user to the local *Hyper-V Administrators*
   group (Computer Management → Local Users and Groups). Log out/in.
   Pegoles detects membership and reports `PermissionMissing` otherwise.
4. **Register the Pegoles socket service** (administrator console, once):
   create key
   `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices\<service-guid>`
   with `ElementName` = `Pegoles Guest Control Plane`, where
   `<service-guid>` is `hyperv_service_guid_for_port(4050)` =
   `00000FD2-facb-11e6-bd58-64006a7986d3`.
   (A future installer performs exactly this; manual registration is
   equivalent and auditable.)
5. **Verify**: run `pegoles-windows-setup check` (or the in-app
   diagnostics): capability probe must report `Supported` with no
   `required_setup` entries. Detection is live OS APIs (registry version/
   edition, `IsProcessorFeaturePresent` VT flag, vmms service state,
   ComputeCore.dll presence, GUID subkey read, admin + Hyper-V-Admins
   token checks, reboot markers) — anything unprovable stays Unknown,
   never assumed. Then Pegoles runs unprivileged.

## Never

- Never run Pegoles itself as Administrator for daily use.
- Never disable firmware virtualization checks to "make it work".
- Never install Hyper-V on Home via unsupported scripts.
- Never register socket services from the unprivileged runtime.

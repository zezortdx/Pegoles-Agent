# Pegoles Computer — Image Spec

Target: an extremely light Debian appliance the agent lives inside. The user normally sees it through Pegoles Agent, so the guest UI does not need to be beautiful — it needs to be automatable.

## Base

- **Debian 13 minimal**, ARM64 first (Apple Silicon hosts)
- No Ubuntu. No GNOME. No KDE. No XFCE. No Alpine Linux (musl/dev-tooling friction is not worth the megabytes).

## Components

- `systemd`
- Wayland + `weston` (minimal compositor, no full desktop environment)
- `chromium`
- Lightweight terminal (e.g. `foot` or `weston-terminal`)
- `git`, `curl`, `ca-certificates`
- `python3`, `node`
- Pegoles Guest Runtime (agent I/O: screenshots, input injection, file/shell execution inside the guest)

## Non-goals for the image

- No login manager / display manager theming
- No office / media apps
- No cloud agents or telemetry

## Running system (macOS Apple Silicon)

`MacOSVirtualizationBackend` drives Virtualization.framework through
`pegoles-vm-host`. Guest: official `debian-13-nocloud-arm64` cloud image
(EFI boot, headless, no network), derived into Pegoles Base Image v0.1
with the guest runtime + systemd unit. Default: **2 vCPU / 1536 MB RAM**.
Serial console captured to `logs/serial.log`. `Running` means the
hypervisor started the VM; guest readiness is a separate vsock
handshake, never faked. Details: `docs/DEBIAN_IMAGE.md`.

## Phase 3.5 status: logical image vs platform artifacts

"Pegoles Base Image v0.1" is a LOGICAL image (Debian 13 + guest runtime
0.1 + protocol 1). Each host platform gets its own ARTIFACT with its own
checksum — hashes are never shared across artifacts:

- macOS / arm64: `pegoles-debian-13-arm64.raw` — BUILT, verified, booted.
- Windows / amd64: `pegoles-debian-13-amd64.vhdx` — PLANNED (matrix +
  manifest model ready; not built in 3.5).

The manifest (`manifest.json`) records the logical identity plus one
`artifacts[]` entry per built platform (file name, format, sha512,
bytes). See `PLATFORM_MATRIX.md` and `image.rs::known_artifacts()`.
Guest kernel gate: universal images need `CONFIG_VSOCKETS` plus BOTH
`CONFIG_VIRTIO_VSOCKETS` and `CONFIG_HYPERV_VSOCKETS` (y or m);
`check-kernel.sh --strict-universal` enforces it. The current arm64
kernel already passes (`universal_ready: true`).

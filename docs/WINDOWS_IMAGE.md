# Windows Image (Debian 13 amd64 → VHDX)

> **Superseded (2026-09-27).** This describes the Phase 3.6 image path (an unprovisioned VHDX provisioned on first boot on the PC). The current design is in `docs/WINDOWS_ARCHITECTURE.md` (Host Compute System with only the Virtual Machine Platform, a small broker service, the guest runtime listening on vsock 850, an image provisioned at build time by `scripts/build-guest-image/build-x64.sh`). Kept for history.

How the Windows artifact is produced, verified, and recorded. Build
machine only (macOS/Linux with Docker + qemu-img + e2fsprogs); the
Windows runtime never needs any of these tools.

## Pipeline

```text
official debian-13-generic-amd64.tar.xz (cdimage.debian.org, SHA-512 verified)
  → extract RAW (system tar, like macOS flow)
  → qemu-img convert -f raw -O vhdx -o subformat=dynamic disk.vhdx
  → kernel gate (debugfs read-only + check_kernel_config --strict-universal)
  → publish into images/pegoles-base-0.1/ as disk.vhdx (own hash, own record)
```

Reproduce: `cargo run -p pegoles-computer --example build_windows_artifact`
(needs `qemu-img` and e2fsprogs `debugfs` on PATH; fails closed otherwise).

## Measured (2026-09-21, build machine)

- Source: `debian-13-generic-amd64.tar.xz`, 320,566,748 bytes, SHA-512
  verified against official `SHA512SUMS`.
- VHDX: dynamic subformat, 3.0 GB logical → ~1.3 GB physical,
  sha512 `e5c43b89…` (recorded in manifest; verify with
  `sha512sum disk.vhdx`).
- Kernel `6.12.107+deb13-amd64`: `vsock`/`virtio_vsock`/`hyperv_vsock`
  all present as modules → `universal_ready: true`.

## x64-0.2: browser

`pegoles-base-x64-0.2` adds Chromium locked to the host proxy. Same files
as arm64 (`seed/browser`, `seed/weston`, browser and forwarder units);
`build-x64.sh` already provisions with cloud-init, so the debs, the
`pegoles-egress` user and the policy tree land in the provisioning boot and
`patch-image.sh` only refreshes files and asserts ownership. The forwarder
is built for x86_64 by `build-runtime.sh`. Details, policy list and the
checks to run on hardware: `docs/DEBIAN_IMAGE.md` ("0.4 / x64-0.2: browser")
and `docs/EGRESS.md`. The catalog pin (`images.json`) and
`manifests/pegoles-base-x64-0.2.packages.txt` are added when the image is
built.

## Non-determinism note

VHDX container bytes embed creation metadata (GUID/timestamps), so two
conversions of identical content differ byte-wise (observed: two runs,
two hashes, same content). Reproducibility = same inputs + same process,
verified by converting + kernel-gating + booting — never by comparing
bytes across builds. Each publish records the hash OF THOSE BYTES.

## Provisioning state: UNPROVISIONED (honest)

`disk.vhdx` contains stock Debian 13 amd64 + cloud-init, NO guest
runtime yet (manifest: `guest_runtime_version:
"pending-first-boot-provisioning"`). Provisioning without Hyper-V
hardware would mean blind filesystem surgery (debugfs writes without
boot verification) — refused. On Windows hardware, first boot attaches
the NoCloud seed ISO (built by `scripts/build-guest-image/`, amd64
runtime binary cross-built via Docker) and cloud-init installs exactly
like the proven macOS flow, then the disk is sealed as provisioned.
Until then: manifest says so, `boot_source` serves it, and GuestReady
can only complete after that first provisioning boot.

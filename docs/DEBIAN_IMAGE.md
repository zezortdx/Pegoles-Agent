# Debian Image (Phase 2)

## Source (only)

- Base URL (TLS): `https://cdimage.debian.org/images/cloud/trixie/latest/`
- Artifact: `debian-13-nocloud-arm64.tar.xz` (~271 MB, extracts to `.raw`)
- Checksums: `SHA512SUMS` in the same directory
- Why `nocloud`: no cloud-init, boots straight to a root prompt — no
  network and no metadata service required, matching our disabled-network VM.

No mirrors, no Ubuntu, no Alpine. QEMU is not involved at any step: the
`.raw` is consumed directly by `VZDiskImageStorageDeviceAttachment`.

## Verification (fail closed)

1. Download `SHA512SUMS` over TLS from the base URL.
2. Parse the line for the artifact; a missing entry aborts.
3. Download the artifact with byte progress.
4. Stream SHA-512 over the file; on mismatch **delete the file** and return
   `ImageVerificationFailed`. Unverified bytes never boot (`load()` only
   accepts `Ready`, i.e. raw + `.verified` marker present).

Checksum strength follows Debian's guidance for these directories
(TLS + DNSSEC-secured official host); see `docs/PEGOLES_COMPUTER.md`.

## Storage

```text
~/Library/Application Support/Pegoles/
  images/pegoles-debian-13-arm64/
    base.raw            # verified, read-only (0444)
    base.raw.verified   # hex digest marker
  computers/<computer-id>/
    metadata.json
    disk.img            # private writable copy of base.raw
    efi-vars.bin        # per-computer EFI variable store
    machine-id          # stable VZGenericMachineIdentifier
    logs/serial.log     # virtio serial console output
```

The base is never modified; per-computer copies are plain `cp` (copy-on-write later).

## Prepare flow

UI `[ Prepare Computer ]` -> `prepare_image` (background thread, real
`downloaded/total` progress on `pegoles://image-progress`) -> Ready.
Manual equivalent:

```bash
PEGOLES_DATA_DIR=/tmp/pegoles-data cargo test -p pegoles-computer
```

## Real-hardware smoke test

Requires macOS arm64 + the helper built + image prepared:

```bash
cd native/macos/pegoles-vm-host && swift build -c release && cd -
bash scripts/codesign-dev.sh   # ad-hoc entitlement for helper + `tauri dev`
pnpm --filter @pegoles/desktop tauri dev  # Prepare Computer -> Create -> Start
PEGOLES_VM_HOST="$PWD/native/macos/pegoles-vm-host/.build/release/pegoles-vm-host" \
  PEGOLES_REAL_VM_TEST=1 cargo test -p pegoles-computer real_vm -- --nocapture
```

Expected: create artifacts -> Apple validation passes -> Debian boots
(Vz reports running) -> pause/resume/stop real -> `serial.log` captured.
`Running` means Virtualization.framework started the VM; guest-level
readiness (`PEGOLES_GUEST_READY`) is explicitly a later phase — see §16
of the Phase 2 brief and `docs/PEGOLES_COMPUTER.md`.

## 0.4 / x64-0.2: browser

Files only so far; the images are not built yet (`images.json` pins are
unchanged, package manifests `manifests/pegoles-base-0.4.packages.txt` and
`pegoles-base-x64-0.2.packages.txt` come from the build's dpkg status).
Contract: `docs/EGRESS.md`.

- **Packages** (`build-deb-bundle.sh`, exact pins): `chromium`
  150.0.7871.181-1~deb13u1 (trixie-security), `fonts-liberation`,
  `ca-certificates`. `chromium-sandbox` (setuid) is not installed and the
  build fails if it appears: the browser uses the user-namespace sandbox,
  under `NoNewPrivileges`. Debs can only be installed by the cloud-init
  provisioning boot; `patch-image.sh` (debugfs) cannot, so it refuses a
  source disk without `/usr/bin/chromium` or the `pegoles-egress` user.
- **Launcher**: `weston.ini` (now `seed/weston/weston.ini`, installed by both
  paths) has a thin top panel with one launcher. It runs
  `/usr/local/bin/pegoles-open-browser`, which touches
  `/run/pegoles/launch-browser`; `pegoles-browser.path` starts the hardened
  `pegoles-browser.service` (tmpfs profile/downloads in
  `/run/pegoles-browser`, wiped on close, no AF_VSOCK, loopback-only IP).
  Earlier images had `panel-location=none`.
- **Users and files**: system user `pegoles-egress` (nologin, no home, no
  groups) runs `pegoles-egress-forwarder.service` (binary
  `/usr/local/lib/pegoles/pegoles-egress-forwarder`, built by
  `build-runtime.sh`). `/etc/chromium`, `policies`, `managed` are
  `root:root 0755`; `pegoles.json` is `root:root 0644`;
  `pegoles-egress-ca.json` is `pegoles-egress 0644` (pre-created with
  `{"CACertificates": []}`) and the only file the forwarder unit may write
  (`ReadWritePaths` on that file). The agent's user can write nothing there;
  `patch-image.sh` asserts modes/owners and that the dir holds only those
  two files. Chromium merges the files by name and the CA file sorts before
  `pegoles.json`, so `pegoles.json` wins any key both define; the
  forwarder could still add keys `pegoles.json` does not set, so the list
  below sets everything that matters.
- **CA**: policy `CACertificates` (list of base64 DER; `chrome.*` 132+, so
  Linux; trixie ships 150). Fallback if the hardware test shows the Debian
  build ignores it: NSS database of the browser user (`certutil -A -t "C,,"`
  into `sql:$HOME/.pki/nssdb`, where `HOME` is the unit's tmpfs, so the
  forwarder would hand the DER to a root helper; not built).
- **Policy names** were checked against Chromium's policy definitions; the
  deprecated top-level `ProxyMode`/`ProxyServer`/`ProxyBypassList` are
  replaced by the `ProxySettings` dict; `PrivacySandbox*Enabled`,
  `PromotionalTabsEnabled`, `WelcomePageOnOSUpgradeEnabled` are deprecated
  or not on Linux and are not used. `AutofillAddressEnabled` and
  `AutofillCreditCardEnabled` are deprecated (M156) but are what Chromium
  150 understands; `AutofillSettings` (`*`, `all`) is set too for later
  versions.
- **Decisions**: `SafeBrowsingProtectionLevel` 0 because Safe Browsing needs
  Google endpoints (unreachable in allowlist mode, a leak of visited URL
  hash prefixes in open_web) and the host already enforces its own signed
  threat snapshot and response inspection. `DownloadRestrictions` 1 (the
  host allows only PDF/images/text anyway). `AllowFileSelectionDialogs`
  false: no reading guest files into pages, downloads go to the fixed
  directory without a prompt. `chrome://*`, `file://*`, `devtools://*`,
  `view-source:` are blocked, no `chrome://` exception (new tab, home and
  startup are `about:blank`; there is no Google NTP or search provider).
  `chrome-extension:`/`chrome-untrusted:` are not blocked: the built-in PDF
  viewer needs them. `SSLErrorOverrideAllowed` false (a bad certificate is a
  deny, as on the host). Loopback is sent to the proxy (`<-loopback>`),
  which denies it.
- **To check on hardware**: PDF opens; typing in pages works through
  uinput; Chromium starts with `/run/pegoles` read-only (else add it to
  `ReadWritePaths` of the browser unit); the `CACertificates` policy is
  live-reloaded and trusted (`https://` page through the proxy);
  `systemd-analyze security pegoles-egress-forwarder pegoles-browser`;
  the top panel does not break the input fixture or screenshot mapping.

## Measured on real hardware (2026-09-20, MacBook Pro Apple Silicon, macOS 26.5)

- Artifact: `debian-13-nocloud-arm64.tar.xz`, 283,733,116 bytes
- SHA-512 (from official `SHA512SUMS`, verified locally):
  `49603bde…31a293ac688b2351` (full value in the `.verified` marker file)
- Extracted `base.raw`: 3,221,225,472 bytes, GPT + FAT16 ESP containing
  `EFI/BOOT/BOOTAA64.EFI` (shim), `GRUBAA64.EFI`, `GRUB.CFG` chainloading
  the root partition's grub config — complete Debian ARM64 EFI boot chain.
- `create` + Apple `validate()`: ~145 ms
- `start` accepted (hypervisor Running): ~95 ms
- Full lifecycle verified: start → running → pause → paused →
  resume → running → stop → stopped (all native, all asserted in test)
- Serial console: mechanism verified (file attachment created, 0 write
  errors); guest emitted **0 bytes in 30 s**. The nocloud cloud image does
  not configure an `hvc0` (virtio-console) kernel console, so there is
  nothing to capture yet. No readiness is claimed from this; the
  guest-side console marker is Phase 3 work.
- RAM: helper process ~11 MiB RSS while VM runs + 1536 MiB guest
  allocation managed by the hypervisor. No sudo used at any point.

## Guest control plane over the derived image (Phase 3, re-verified 3.5)

End-to-end (`PEGOLES_REAL_GUEST_TEST=1`, same machine):

- VM start → Running: ~95 ms
- Guest boot + systemd + runtime vsock dial + versioned handshake → Ready: **~6 s**
- Ping → Pong: **0 ms** (vsock loopback-class latency)
- SystemInfo: `debian 13, kernel 6.12.107+deb13-arm64, aarch64,
  host=pegoles, runtime 0.1.0, proto 1` — all asserted, none invented
- Stop → Stopped: ok
- Helper RSS with running VM + connected guest: ~9 MiB
- Guest kernel gate (`check-kernel.sh --strict-universal`): `vsock`,
  `virtio_vsock` and `hyperv_vsock` all present as modules →
  `universal_ready: true` (same Debian 13 arm64 kernel serves future
  Hyper-V guests too)

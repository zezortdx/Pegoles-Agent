# Changelog

All notable changes to Pegoles Agent are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Release tags are `v<version>`; the app, Cargo and npm versions must match
the tag (the release workflow refuses a mismatch).

## [Unreleased]

### Added

- **Source install.** `./scripts/install.sh` builds Pegoles from a clone
  and installs `~/Applications/Pegoles.app`. It needs only the Xcode
  Command Line Tools: Rust, Node.js, pnpm and cargo-about are downloaded
  into the checkout, each pinned by version and SHA-256 or SHA-512. It
  runs in a clean environment and never uses sudo. It refuses quarantined
  checkouts, and cargo configurations above the checkout that another
  user could have planted. It stages the new app next to the old one and
  swaps it in only after the signature verifies.
- **Hosted computer image.** `pegoles-base-0.3` is downloaded from the
  immutable GitHub release `guest-image-0.3`; the pinned digests are
  unchanged.
- GitHub social preview and README screenshots; an architecture overview
  that matches the code.

### Fixed

- Verification scripts could pass a check on a match: tool output piped
  into `grep -q` or `head` under `pipefail` failed the pipeline on the
  first match. This affected the bundle verifier's build-path checks, the
  image sanitizer's key and seed proof, and the installer's
  "Pegoles is running" check. The shipped app and the published image
  were re-verified.
- The guest runtime's `memfd_create` call compiled only on aarch64 (found
  by the x86_64 CI job).
- Found by the installed-app end-to-end runs: a guest that stopped
  responding left the app on "Starting…" forever (it now becomes an error
  after 60 s without a reconnect); Settings showed "Runs on mac_o_s";
  image setup errors were unreadable; the installer broke license notices
  in a developer checkout without a message, and asked for 15 GB of disk
  (the build peaks at about 5 GB; it now asks for 8 GB).

### Changed

- The binary release workflow runs only when `PEGOLES_BINARY_RELEASES` is
  enabled (0.1.x is source-first; Developer ID signing is deferred).
- Commit history uses the maintainer's GitHub noreply address.

## [0.1.0-rc.1] - not yet tagged

First release candidate. macOS on Apple silicon only; tested on one Mac
(M4 Pro, 24 GB, macOS 26.5). Not yet Developer ID signed or notarized.

### Added

- **Isolated computer.** A Linux VM per computer through Apple's
  Virtualization.framework, driven by a Swift helper (`pegoles-vm-host`)
  that exits with the app: no network device, no shared folders, no
  clipboard, no host input devices. Each computer is an APFS clone of a
  sealed image; reset restores the image and a fresh EFI variable store.
- **Guest image v0.3** (Debian 13 arm64, weston, foot) with guest runtime
  0.2.0: vsock channel authenticated by reserved source port, uinput input,
  weston screen capture, non-dumpable runtime; ssh, network, apt and
  update services masked; SSH host keys and the random seed removed
  before sealing.
- **In-app computer setup.** "Set up computer" downloads the 562 MB image
  once (resumable, HTTPS), checks it against the archive SHA-256 and disk
  SHA-512 compiled into the app, and installs it atomically; release
  builds boot only that pinned image and re-hash it once per session.
- **Typed actions and policy.** The model can only observe, point, type,
  press keys and wait inside the VM. `pegoles-policy::evaluate` is an
  exhaustive, deterministic match (coordinates, caps, key vocabulary,
  control and invisible/bidi characters, key-material tripwire); every
  action passes it and Core's executor (rate limit, control arbitration).
- **Pegoles Local**, the default planner: a small vision-language model on
  the Mac (MLX), no API key, offline once installed. Default MAI-UI-2B
  6-bit, chosen on a 22-task benchmark on the real VM. Its Python/MLX
  runtime ships inside the app (reproducible, hash-locked); the worker
  runs in a `sandbox-exec` profile with no network, fork, exec, Apple
  Events, pasteboard, Keychain or LaunchServices access; its output is
  parsed by a strict parser; models come from a pinned catalog and a
  verified store.
- **Optional Claude planner** (Anthropic API) behind the same provider
  switch and policy path; switching to it and entering the key happen in
  native macOS dialogs, and the key lives in the Keychain.
- **Agent orchestrator** (`pegoles-agent`): turn/action/screenshot/time
  budgets with a watchdog, batch halt, cancellation, loop and oscillation
  brakes, worker crash recovery.
- **Desktop app** (Tauri 2 + React): run and stop tasks, watch the VM and
  the agent's narration, Intelligence settings, in-app setup of the model
  and the computer image with progress, reset and remove. App ACL (only
  the commands the UI uses), navigation guard, strict CSP; developer-only
  commands are compiled out of release builds.
- **Packaging**: build and sign stages, inside-out Developer ID signing
  with hardened runtime and timestamps, the virtualization entitlement
  only on the helper, DMG, third-party license notices in the bundle,
  `scripts/release/verify-artifact.sh`, notarization script.
- **Release engineering**: CI on macOS arm64 (Rust incl. the Tauri shell,
  release-profile tests, Swift helper, frontend) plus portability of the
  shared crates; SHA-pinned actions with read-only tokens; `cargo deny`,
  `cargo audit`, `pnpm audit`, dependency review, CodeQL, Dependabot; a
  tag-triggered release workflow whose signing job runs no dependency
  code, notarizes, staples, verifies, writes `SHA256SUMS`, a build
  manifest and a CycloneDX SBOM, attests build provenance and drafts the
  GitHub release.
- Security policy, contributing guide, code of conduct, threat model
  (`docs/THREAT_MODEL.md`), privacy notes (`docs/PRIVACY.md`), release
  gates (`docs/RELEASE_GATES.md`), hardware soak harness.

### Changed

- The action vocabulary no longer contains `shell`, `read_file`,
  `write_file` or `open_url`: they were never implemented in the guest and
  used to report false success.

### Fixed

- A guest flooding `GuestHello` frames could overflow the app's pump
  stack and crash it; outbound work is now bounded and non-reentrant, and
  every guest connection must handshake.
- The model worker supervisor could hang on a worker that closed its
  output or stopped reading, and buffer unbounded stderr.
- Guest compositor memory leak during screen capture (a memfd per
  capture).
- Keypress taps, first-click hit-testing and app-level script
  interruption on the real VM.
- Sealing refuses truncated work disks.

### Security

- Findings of an independent adversarial review (host/VM escape,
  malicious model, web/Tauri, supply chain) fixed; residual risks are
  listed in `docs/THREAT_MODEL.md`.
- Guest image build tooling: container images pinned by digest, the
  repository mounted read-only into build containers, no privileged
  containers, private temp directories instead of fixed `/tmp` paths.

### Known limitations

- Not yet signed with a Developer ID or notarized; no published build.
- Tested on one Mac (M4 Pro, 24 GB); no minimum RAM is claimed.
- The Claude planner is unit-tested but has not been run against the live
  API for this release.
- The agent's computer has no network; no access to the Mac's files.
- No native VM view or human "take control" input yet; no auto-update.
- Windows code is not compiled or verified; there is no Linux host backend.

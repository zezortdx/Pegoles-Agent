# Pegoles Agent

**AI gets its own computer. Not your computer.**

Pegoles is a computer-use agent for macOS. You give it an objective; a
model plans; every action it proposes is a typed, policy-checked input to
an isolated Linux virtual machine that has no network, no shared folders
and no clipboard. You watch that machine and the agent's narration, stop
it at any time, and reset the machine to a sealed image.

No API key is needed. The default planner, **Pegoles Local**, is a small
vision-language model that runs on your Mac. A cloud planner (Claude,
through your own Anthropic API key) is optional.

## Status: 0.1.0-rc.1 (release candidate, not yet published)

| | |
|---|---|
| Platform | macOS on Apple silicon only. Built for macOS 14 or later; tested on macOS 26.5 only. No Windows or Linux host support. |
| Hardware tested | One Mac: M4 Pro, 24 GB. Pegoles Local peaks at about 4.5–5 GB of memory (model worker ≈ 4 GB, VM ≈ 0.5 GB). Smaller Macs are untested; no minimum RAM is claimed yet. |
| Disk | About 6 GB: the app (≈ 0.5 GB), the computer image (3 GB installed, 562 MB download) and the default model (2.2 GB). |
| Distribution | Signed, notarized builds are not published yet: this candidate still needs a Developer ID signature and Apple notarization (see [docs/RELEASE_GATES.md](docs/RELEASE_GATES.md)). |

What works today, verified on real hardware:

- The isolated computer boots from the sealed image in about 3.4 s; the
  agent clicks, types, presses keys, scrolls, drags and looks at the
  screen through the policy; Stop cancels a run within milliseconds to
  about 3 s; a guest runtime crash recovers in about 3 s; reset returns
  the machine to the sealed image.
- Pegoles Local (MAI-UI-2B 6-bit on MLX) completes natural-language tasks
  with no API key and no network, chosen on a 22-task benchmark on the
  real VM (17/22 goals; [benchmarks](benchmarks/local-models/README.md)).
  It is a 2-billion-parameter model: it succeeds on short, concrete
  terminal and UI tasks and fails on some; expect retries.
- Its Python/MLX runtime ships inside the app; the model and the computer
  image are downloaded once in the app and checked against digests built
  into it.

What does not exist yet (see [Known limitations](#known-limitations)):
web browsing or any network access for the agent, access to your Mac's
files or apps, a live view of the VM you can control with your own mouse
and keyboard, and saved task history across restarts.

## Install (once a release is published)

1. Download `Pegoles_<version>_arm64.dmg` from GitHub Releases and verify it:
   ```bash
   shasum -a 256 -c SHA256SUMS --ignore-missing
   gh attestation verify Pegoles_<version>_arm64.dmg --repo zezortdx/Pegoles-Agent
   ```
2. Open the DMG and drag **Pegoles Agent** to Applications.
3. On first launch, choose **Set up computer** (downloads and checks the
   562 MB computer image) and set up **Pegoles Local** in Settings
   (downloads and checks the 2.2 GB model). After that it works offline.

To uninstall: quit Pegoles, delete the app, and delete
`~/Library/Application Support/Pegoles`.

## Security model, briefly

A model is an untrusted planner, local or cloud alike. It can only
propose *observe, point, type, press keys, wait*; there is no host shell,
file, process or URL action anywhere in the protocol. Every action passes
a deterministic, exhaustive policy and Core's executor, and lands in a VM
with no network device. The local model runs in its own sandboxed process
(no network, no other processes, nothing in your home folder beyond its model and runtime)
and its output is parsed strictly. The desktop webview can call only the
commands the UI needs, has no network access at all, cannot navigate
away, and cannot switch you to a cloud planner or store a key without a
native macOS confirmation.

- Design and enforcement: [docs/SECURITY.md](docs/SECURITY.md)
- Threats, controls and residual risks: [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md)
- What leaves your Mac: [docs/PRIVACY.md](docs/PRIVACY.md) (nothing, with
  Pegoles Local, after the one-time downloads)
- Reporting vulnerabilities: [SECURITY.md](SECURITY.md)

## Development

Requirements: Rust (CI uses 1.97.1), Node 20–26 with pnpm 9, Xcode or the
Command Line Tools (Swift), an Apple silicon Mac. Building guest images
also needs Docker and e2fsprogs. Details in [CONTRIBUTING.md](CONTRIBUTING.md).

```bash
pnpm install --frozen-lockfile --ignore-scripts
bash scripts/check.sh                              # every gate (frontend, fmt, clippy, tests, Swift)

# VM helper with its virtualization entitlement (after every Swift build)
swift build -c release --package-path native/macos/pegoles-vm-host
bash scripts/codesign-dev.sh

# Pegoles Local: the shipped runtime (reproducible) and the default model
bash scripts/local-model/build-runtime.sh
cargo run --release -p pegoles-inference --example models -- install mai-ui-2b-6bit

# Desktop app in development (Vite on port 1430)
pnpm --filter @pegoles/desktop tauri dev

# Real-hardware end to end (release build, helper beside the binary)
cargo build --release -p pegoles-agent --example agent_e2e
cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
./target/release/examples/agent_e2e

# App bundle + DMG (ad-hoc signed without PEGOLES_SIGN_IDENTITY)
bash scripts/package-macos.sh
```

The computer image is built by `scripts/build-guest-image` (see
[CLAUDE.md](CLAUDE.md) for the patch → sanitize → seal loop); the app
itself installs the published, pinned image.

## Layout

```text
apps/desktop               Tauri 2 shell (Rust commands, ACL) + React UI
crates/pegoles-agent       orchestrator: runner, budgets, planners (local, Claude, scripted)
crates/pegoles-inference   hardware probe, pinned model store, sandboxed MLX worker backend
crates/pegoles-core        computer registry, executor (policy → control → input), tasks, events
crates/pegoles-policy      deterministic action policy
crates/pegoles-protocol    shared types (actions, events, tasks, limits, text rules)
crates/pegoles-computer    macOS VM engine, guest session, pinned image distribution
crates/pegoles-guest-proto host ↔ guest protocol (JSONL over vsock)
guest/runtime              Linux guest runtime (vsock, uinput, weston capture)
native/macos               Swift VM helper (Virtualization.framework)
native/windows             Windows HCS helper (never compiled; not supported)
workers/mlx                the model worker (Python, sandboxed, offline) and its lock
benchmarks/local-models    real-VM model benchmark
scripts/                   gates, packaging, release, runtime and image builders
```

## Known limitations

- Pre-release: not yet Developer ID signed or notarized; no published
  build. Tested on one Mac (M4 Pro, 24 GB, macOS 26.5).
- The agent's computer has no network: no web browsing, no downloads, no
  package installs inside the VM.
- No access to your Mac's files, screen or apps — by design.
- No live, human-controllable view of the VM yet (you see snapshots and
  the agent's narration; "take control" does not route your input).
- Pegoles Local is a small model: it completes short concrete tasks and
  gets some wrong; the loop brake and budgets stop runaway runs.
- The Claude planner is unit-tested but was not run against the live API
  for this release.
- No auto-update in 0.1: install new versions from GitHub Releases.
- Task history is kept in memory only.

## License

MIT (see [LICENSE](LICENSE)). Third-party components shipped in the app
(Python, MLX, and the other runtime packages) keep their own licenses;
see [docs/RELEASE_GATES.md](docs/RELEASE_GATES.md) for the inventory.

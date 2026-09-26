# Pegoles Agent

**AI gets its own computer. Not your computer.**

Pegoles is a computer-use agent for macOS. You give it an objective; a
model plans; every action it proposes is a typed, policy-checked input
to an isolated Linux VM (no network, no shared folders, no clipboard).
You watch the VM and the agent's activity, stop it at any time, and
reset the VM to a sealed image.

## Status (2026-09-26)

- macOS Apple Silicon is the reference platform and the only supported
  one. Windows code exists but is unverified; Linux has no backend.
  Details: [docs/PLATFORM_MATRIX.md](docs/PLATFORM_MATRIX.md).
- Real end to end on hardware: boot → authenticated guest channel →
  orchestrated task (click, type, keys, observe, pixel verification) →
  cancel → guest-runtime crash recovery → reset/teardown → second boot
  (`crates/pegoles-agent/examples/agent_e2e.rs`).
- The model planner uses Claude through the Anthropic API (key in the
  macOS Keychain). It is unit-tested but has not been run against the
  live API from this environment.
- Security model and residual risks: [docs/SECURITY.md](docs/SECURITY.md).
  Engineering state and next steps: [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md).

## Run (development)

```bash
bash scripts/setup.sh
bash scripts/check.sh                                  # all gates

# VM helper (Swift) + virtualization entitlement for dev binaries
(cd native/macos/pegoles-vm-host && swift build -c release)
cargo build -p pegoles-desktop && bash scripts/codesign-dev.sh

# Guest image: see scripts/build-guest-image (build or patch, then seal)
# Desktop app (dev)
pnpm --filter @pegoles/desktop tauri dev

# Real hardware E2E (release; helper beside the binary, as in a bundle)
cargo build --release -p pegoles-agent --example agent_e2e
cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
./target/release/examples/agent_e2e
```

Requires: Rust stable, Node ≥ 20, pnpm 9, Xcode command line tools
(Swift), Apple Silicon Mac for VMs. Image building also needs Docker,
qemu and e2fsprogs.

## Layout

```text
apps/desktop           Tauri 2 shell (Rust commands) + React UI
crates/pegoles-agent   orchestrator: runner, budgets, Claude planner, scripted planner
crates/pegoles-core    computer registry, executor (policy → control → input), tasks, events
crates/pegoles-policy  deterministic action policy
crates/pegoles-protocol shared types (actions, events, tasks, limits)
crates/pegoles-computer VM backends (macOS helper engine, Windows HCS), images, guest session
crates/pegoles-guest-proto host ↔ guest protocol (JSONL over vsock)
guest/runtime          Linux guest runtime (vsock, uinput, weston capture)
native/macos           Swift VM helper (Virtualization.framework)
native/windows         Windows HCS helper (unverified)
scripts/               check.sh, bench.sh, build-guest-image/
```

MIT licensed.

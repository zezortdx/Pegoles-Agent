# Pegoles — Project State

Canonical, concise engineering state. Update it when reality changes; do
not turn it into a diary. Last full audit and RC pass: 2026-09-26; local-first model phase: 2026-09-26;
release hardening for 0.1.0-rc.1: 2026-09-26 (gates and evidence: `docs/RELEASE_GATES.md`).

## What Pegoles is

A secure computer-use agent. A planner proposes typed actions; a
deterministic policy and Core's executor apply them to an isolated macOS
VM with no network, no shared folders and no clipboard. The default
planner is **Pegoles Local**: a small vision-language model running on
the Mac (MLX), no API key, works offline once installed. Claude via the
Anthropic API is an optional alternative; a deterministic script drives
the hardware E2E. The user watches the VM and the agent's narration,
stops runs, and resets the VM to a sealed image. Model architecture:
`docs/MODEL_ARCHITECTURE.md`.

## Architecture (as built)

```text
React UI (apps/desktop/src)        renders; never renders model text as HTML
  │ Tauri commands / pegoles://event
Tauri shell (src-tauri)            commands, status cache, background pump,
                                   agent supervisor (one run), Keychain key
  │
pegoles-agent      runner (budgets, batch halt, cancellation), planners:
                   local (Pegoles Local: prompts, strict parser, loop
                   brake, crash recovery), anthropic (optional, raw
                   HTTPS), scripted (E2E); CoreComputer = product bridge
pegoles-inference  hardware/memory, pinned model catalog, verified
                   store (resume, SHA-256, atomic install), backend
                   trait, MLX worker supervisor
workers/mlx        persistent Python MLX worker (sandboxed, offline)
pegoles-core       ComputerRegistry (lifecycle, guest pump, executor:
                   rate limit → policy → control → input ops), tasks, bus
pegoles-policy     exhaustive evaluate(): observe/pointer/keyboard/wait
pegoles-protocol   typed actions/events/tasks/limits (no host verbs)
pegoles-computer   macOS engine → Swift helper (JSONL, bounded, killed on
                   hang), guest session, images, computer store (resume /
                   clean, flock)
pegoles-guest-proto JSONL over vsock, 64 KiB frames
native/macos/pegoles-vm-host  Swift, Virtualization.framework; exits with
                   its parent; authenticates the guest by reserved port
guest/runtime      Linux runtime 0.2.0: reserved-port vsock client,
                   uinput, weston capture (memfd closed per capture),
                   non-dumpable
image v0.3         Debian 13 arm64 + weston + foot terminal (workspace),
                   ssh/network/apt services masked; sanitized (no SSH
                   host keys) and pinned for distribution 2026-09-26
```

## Verified (real hardware, `crates/pegoles-agent/examples/agent_e2e.rs`)

Boot → authenticated guest → orchestrated tasks through the product
path: click, typing (incl. `'"` dead keys, `$()`), Enter, scroll,
double-click, drag, observe, pixel verification (red block, channel
order), cancel (27 ms), forged action blocked by policy, runtime killed
inside the guest → host sees it → recovered (3.2 s), stop/destroy
(0.9 s), second session (3.1 s). Cold boot → agent-ready 3.3–3.7 s;
observe p50 108 ms / p95 119 ms. Numbers: `docs/PERFORMANCE.md`.

### Pegoles Local (2026-09-26)

- Keyless product path (`apps/desktop/src-tauri/examples/local_e2e.rs`,
  the app's own `start_run` + `LocalModels`): no cloud key, provider
  Local by default → verified model → sandboxed MLX worker on the host
  → real VM; a natural-language task completed and verified in the
  guest (35.6 s); Stop during inference → cancelled in 2.6 s; worker
  `kill -9` mid-task → respawned, model reloaded, task completed;
  teardown removes VM and worker; 0 internet sockets from app or worker.
- 22-task benchmark on the real VM for MAI-UI-2B and Qwen3-VL-2B (6-
  and 4-bit, 1440 and 1024 px): default MAI-UI-2B 6-bit, 17/22 goals.
  `benchmarks/local-models/`.
- Hostile scripted model on the real VM: schema escapes and bad
  coordinates never reach the executor; loops stopped by the brake;
  a parser-valid key-material paste blocked by policy.
- Model store: real download from pinned Hugging Face revisions,
  resume after a real interrupted transfer, SHA-256 verification,
  atomic install, local-conversion import.
- Capture soak: 400 screenshots, guest memory flat (after the memfd
  leak fix; before, ~260 captures OOM-killed the compositor).

### Release candidate 0.1.0-rc.1 (2026-09-26)

- Pegoles Local runtime ships inside the app (reproducible build,
  33 hash-locked wheels, CPython 3.12.14 pinned); the worker sandbox also
  denies fork, exec, Apple Events, most Mach services, signals and
  process inspection (17 escape probes pinned by a test).
- Guest image distribution: pinned archive/disk digests compiled in,
  in-app "Set up computer" with resumable verified download, release
  builds boot only the pinned image. The published image is sanitized
  (no SSH host keys). The archive is not hosted yet (catalog URL says
  UNPUBLISHED until it is uploaded to the public repository).
- Security review and red team across host/VM, AI/policy, Tauri/web,
  supply chain; every verified finding fixed or recorded as residual
  (`docs/THREAT_MODEL.md`).
- Hardware, after all fixes: `agent_e2e` green; keyless `local_e2e` green
  3 of 5 runs, including the last on the final code (the other two: the
  2B model's own mistakes, stopped by the brakes as designed); release soak: 12 VM lifecycles,
  1200 observations, 60 inferences with forced worker kills, nothing
  left behind, no growth; 2212-capture soak with flat guest memory (one
  transient 5 s compositor timeout).
- Packaging: inside-out signing, DMG, artifact verifier, third-party
  notices in the bundle; release workflow split so no dependency code
  runs while the signing identity is available.
- Not done (external): Developer ID signing, notarization, Gatekeeper
  on a quarantined download, a clean-machine test, publishing.

## Implemented, not verified live

- Claude planner against the live API (no key in this environment):
  request/response/translation/rollover are unit-tested with fixtures;
  it sits behind the same provider switch as Pegoles Local, now behind a
  native confirmation.
- Desktop UI wiring (run/stop tasks, Intelligence settings, local model
  and computer image setup with progress, reset/remove): vitest +
  backend-less shell lab; the live Tauri window was not visually checked
  in this phase (the packaged app was launched and quit cleanly).
- Physical network disconnect: not performed. Offline evidence: 0
  internet sockets from the app process and worker during a full local
  run; the worker sandbox denies all sockets (probed by test).
- Windows HCS backend: never compiled for Windows here.

## Not implemented

- Native VM view / human "take control" input path (embed is a stub).
- Approval grants (`RequireApproval` is reserved; nothing produces it;
  executor fails closed).
- Persistence of tasks/events across app restarts (the computer itself
  resumes).
- Auto-update (no updater in 0.1).
- Linux host backend; Windows is unverified.
- Hybrid local+cloud planner, OpenAI / Gemini / OpenAI-compatible
  providers (the provider seam is ready; see MODEL_ARCHITECTURE.md).

## Commands

```bash
bash scripts/check.sh                                   # all gates
cargo build --release -p pegoles-agent --example agent_e2e \
  && cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/ \
  && ./target/release/examples/agent_e2e               # hardware E2E
bash scripts/local-model/build-runtime.sh               # shipped Python/MLX runtime
cargo run --release -p pegoles-inference --example models -- install mai-ui-2b-6bit
bash scripts/package-macos.sh                           # app + DMG (ad-hoc without an identity)
bash scripts/release/verify-artifact.sh target/release-artifacts/Pegoles_<v>_arm64.dmg
# image: build-runtime.sh → patch-image.sh → sanitize-image.sh → seal_image (see CLAUDE.md)
# keyless local E2E (real VM, app code path): see CLAUDE.md for the Resources links
# release soak: cargo run --release -p pegoles-agent --example release_soak -- --cycles 12
# model benchmark: benchmarks/local-models/README.md
```

## Remaining, prioritized

1. Release blockers (external): Developer ID certificate and notary
   credentials; `zezortdx/Pegoles-Agent` is private on GitHub Free, so
   rulesets, environment reviewers, secret scanning and attestations wait
   for the switch to public; then host the guest image archive and
   replace UNPUBLISHED in the image catalog; run the release workflow;
   Gatekeeper and a clean-machine test on the exact notarized DMG
   (`docs/GITHUB_RELEASE_CHECKLIST.md`).
2. Measure on 8 GB and 16 GB Macs and set a minimum RAM; only a 24 GB
   M4 Pro was available.
3. Visual QA of the packaged app with a live VM (computer image setup,
   Intelligence settings, consent alerts).
4. Run the Claude planner live (needs a key).
5. Local model quality: MAI-UI's remaining failures are habits (one
   character per step, answering by typing, re-toggling) and 13-px
   targets; also occasional invalid replies on a fresh screen.
6. Separate guest users for GUI apps and the runtime (today: same user;
   mitigated by reserved-port auth + non-dumpable runtime).
7. Native VM view + human input (take control).
8. Windows: compile and run on real hardware before claiming anything.

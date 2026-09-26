# Pegoles — Project State

Canonical, concise engineering state. Update it when reality changes; do
not turn it into a diary. Last full audit and RC pass: 2026-09-26; local-first model phase: 2026-09-26.

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
                   ssh/network/apt services masked (resealed 2026-09-26
                   with the capture-leak fix)
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

## Implemented, not verified live

- Claude planner against the live API (no key in this environment):
  request/response/translation/rollover are unit-tested with fixtures;
  it now sits behind the same provider switch as Pegoles Local.
- Desktop UI wiring (run/stop tasks, Intelligence settings, local model
  setup with progress, reset/remove): vitest + backend-less shell lab
  screenshots; the live Tauri window was not visually checked.
- Physical network disconnect: not performed (it would cut the
  development session). Offline evidence: 0 internet sockets from the
  app process and worker during a full local run; the worker sandbox
  denies TCP and DNS (probed); the VM/helper/capture/input path ran
  under a network-denied sandbox.
- Windows HCS backend: never compiled for Windows here (MSVC headers
  unavailable); fixed two compile errors and two API bugs by inspection.

## Not implemented

- Native VM view / human "take control" input path (embed is a stub).
- Approval grants (`RequireApproval` is reserved; nothing produces it;
  executor fails closed).
- Pegoles image distribution (build locally via
  `scripts/build-guest-image`); persistence of tasks/events across app
  restarts (the computer itself resumes).
- Linux host backend.
- Shipping the Python/MLX runtime with the app (today:
  `scripts/local-model/setup-runtime.sh`, hash-locked; the app reports
  "runtime missing" otherwise), and a hosted source for the local 4-bit
  conversion (not offered anyway).
- Hybrid local+cloud planner, OpenAI / Gemini / OpenAI-compatible
  providers (the provider seam is ready; see MODEL_ARCHITECTURE.md).

## Commands

```bash
bash scripts/check.sh                                   # all gates
cargo build --release -p pegoles-agent --example agent_e2e \
  && cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/ \
  && ./target/release/examples/agent_e2e               # hardware E2E
bash scripts/package-macos.sh                           # .app with helper + worker script
# image: build-runtime.sh → patch-image.sh → seal_image (see CLAUDE.md)
bash scripts/local-model/setup-runtime.sh               # MLX runtime (once)
cargo run --release -p pegoles-inference --example models -- install mai-ui-2b-6bit
# keyless local E2E (real VM, app code path):
cargo build --release -p pegoles-desktop --example local_e2e \
  && cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/ \
  && mkdir -p target/release/examples/workers/mlx \
  && cp workers/mlx/pegoles_mlx_worker.py target/release/examples/workers/mlx/ \
  && ./target/release/examples/local_e2e
# model benchmark: benchmarks/local-models/README.md
```

## Remaining, prioritized

1. Ship the MLX runtime inside the signed app (or a verified runtime
   download) so first-run setup needs no terminal; today only the model
   download is in-app.
2. Measure on 8 GB and 16 GB Macs and set `recommended_min_ram_gb`;
   only a 24 GB M4 Pro was available.
3. Visual QA of the desktop app with a live VM (Intelligence settings
   and in-task setup are new).
4. Run the Claude planner live (needs a key) through the provider
   switch; add a live smoke to the E2E behind an env flag.
5. Local model quality: MAI-UI's remaining failures are habits (one
   character per step, answering by typing, re-toggling) and 13-px
   targets; next levers are prompt/context tuning on the benchmark and
   prompt-prefix KV reuse (~0.8 s/step).
6. Image distribution: publish a signed sealed image and a verified
   download path (today the app fails closed if the image is missing).
7. Separate guest users for GUI apps and the runtime (today: same user;
   mitigated by reserved-port auth + non-dumpable runtime).
8. Native VM view + human input (take control).
9. Windows: compile and run on real hardware before claiming anything.

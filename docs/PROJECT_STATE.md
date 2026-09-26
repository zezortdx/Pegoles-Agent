# Pegoles — Project State

Canonical, concise engineering state. Update it when reality changes; do
not turn it into a diary. Last full audit and RC pass: 2026-09-26.

## What Pegoles is

A secure computer-use agent. A planner (Claude via the Anthropic API, or
a deterministic script) proposes typed actions; a deterministic policy
and Core's executor apply them to an isolated macOS VM with no network,
no shared folders and no clipboard. The user watches the VM and the
agent's narration, stops runs, and resets the VM to a sealed image.

## Architecture (as built)

```text
React UI (apps/desktop/src)        renders; never renders model text as HTML
  │ Tauri commands / pegoles://event
Tauri shell (src-tauri)            commands, status cache, background pump,
                                   agent supervisor (one run), Keychain key
  │
pegoles-agent      runner (budgets, batch halt, cancellation), planners:
                   anthropic (computer_toolset_20260801, raw HTTPS),
                   scripted (E2E); CoreComputer = the product bridge
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
                   uinput, weston capture, non-dumpable
image v0.3         Debian 13 arm64 + weston + foot terminal (workspace),
                   ssh/network/apt services masked
```

## Verified (real hardware, `crates/pegoles-agent/examples/agent_e2e.rs`)

Boot → authenticated guest → orchestrated tasks through the product
path: click, typing (incl. `'"` dead keys, `$()`), Enter, scroll,
double-click, drag, observe, pixel verification (red block, channel
order), cancel (27 ms), forged action blocked by policy, runtime killed
inside the guest → host sees it → recovered (3.2 s), stop/destroy
(0.9 s), second session (3.1 s). Cold boot → agent-ready 3.3–3.7 s;
observe p50 108 ms / p95 119 ms. Numbers: `docs/PERFORMANCE.md`.

## Implemented, not verified live

- Claude planner against the live API (no key in this environment):
  request/response/translation/rollover are unit-tested with fixtures.
- Desktop UI wiring (run/stop tasks, model settings, reset/remove):
  vitest only; the live Tauri window was not visually checked.
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

## Commands

```bash
bash scripts/check.sh                                   # all gates
cargo build --release -p pegoles-agent --example agent_e2e \
  && cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/ \
  && ./target/release/examples/agent_e2e               # hardware E2E
bash scripts/package-macos.sh                           # .app with helper
# image: build-runtime.sh → patch-image.sh → seal_image (see CLAUDE.md)
```

## Remaining, prioritized

1. Run the Claude planner live (needs a key) and tune the system prompt
   and effort on real tasks; add a live smoke to the E2E behind an env
   flag.
2. Visual QA of the desktop app with a live VM (UI wiring is new).
3. Image distribution: publish a signed sealed image and a verified
   download path (today the app fails closed if the image is missing).
4. Separate guest users for GUI apps and the runtime (today: same user;
   mitigated by reserved-port auth + non-dumpable runtime).
5. Native VM view + human input (take control).
6. Windows: compile and run on real hardware before claiming anything.

# Pegoles — Project State

Canonical, concise engineering state. Update it when reality changes; do
not turn it into a diary. Last full audit: 2026-09-26.

## What Pegoles is

A secure computer-use agent: a model proposes structured actions, a
deterministic policy decides, and the actions run inside an isolated VM
(never on the host). The user watches the VM and the agent's activity.

## Architecture (as built)

```text
React UI (apps/desktop/src)          renders only; no core logic
  │ Tauri invoke / pegoles://event
Tauri shell (apps/desktop/src-tauri)  thin commands over AppState (one Mutex)
  │
pegoles-core        ComputerRegistry (lifecycle, guest session pump,
                    executor: rate limit → policy → control → input ops),
                    TaskManager, EventBus (broadcast 256)
pegoles-policy      deterministic evaluate(ActionRequest) → Allow/Deny/…
pegoles-protocol    serde types: ComputerAction, AgentEvent, tasks, limits
pegoles-computer    ComputerBackend trait; macOS backend = child process
                    `pegoles-vm-host` (Swift, JSONL over stdin/stdout);
                    guest session + frame reassembly; image manager
pegoles-guest-proto JSONL guest protocol (64 KiB frames) shared with guest
native/macos/pegoles-vm-host   Swift, Virtualization.framework: EFI, 1 disk,
                    virtio-gpu 1440x900, vsock, balloon, entropy, serial
                    (output only). NO network, NO shared dirs, NO clipboard.
guest/runtime       Linux arm64 runtime (user `pegoles`, non-root): vsock
                    client, uinput touchscreen+keyboard, weston capture
Guest image         Debian 13 arm64 + weston (DRM, pixman) + foot, sealed
                    offline from a seed ISO (scripts/build-guest-image)
```

## Working (verified on Apple Silicon hardware, 2026-09-23)

- VM lifecycle create/start/pause/resume/stop via the Swift helper.
- Guest protocol handshake/heartbeat/reconnect over vsock.
- Image v0.2: real input (click/drag/scroll/keys/typing) and real frame
  capture through policy (`crates/pegoles-core/examples/v2_e2e.rs`).

## Audit findings (2026-09-26) — status tracked in the Remaining list

Critical path defects found:
1. Swift helper never exits on stdin EOF; app never kills it on exit →
   orphaned helpers + running VMs (6 found live, cleaned up).
2. No agent orchestrator / model runner exists; tasks stay Pending.
3. Guest double channel swap → host frames have red/blue swapped.
4. Default image is v0.1 (no input/frame caps); if the requested image is
   missing the backend silently boots vanilla Debian.
5. Guest→host: `truncate` on a non-char boundary panics while holding the
   app lock (poisoned → app unusable); unbounded helper→Rust channel;
   blocking guest writes can wedge the helper's only command thread.
6. vsock control channel unauthenticated, newest connection wins.
7. `Shell/ReadFile/WriteFile/OpenUrl` were Allow-able but lowered to zero
   ops and reported "Executed" (false success); string-heuristic policy.
8. Global `Mutex<AppState>` held across long actions/captures; guest
   heartbeat pumping only happens when the UI polls.
9. Reset does not restore the disk; failed create leaks disk dirs.
10. Tauri CSP null; dev-only commands exposed in release builds.

## Platform status

| Platform | Status |
|---|---|
| macOS Apple Silicon | Reference platform. Real VM + guest + computer use. |
| Windows (HCS) | Code exists behind `cfg(windows)`; never booted; vm-host has compile errors on Windows. Not supported. |
| Linux host | Stub only (Mock backend). Not supported. |

## Commands

```bash
bash scripts/check.sh                       # fmt + clippy + test + web gates
pnpm --filter @pegoles/desktop test         # vitest
cd native/macos/pegoles-vm-host && swift build -c release
bash scripts/codesign-dev.sh                # VZ entitlement for dev binaries
```

## Remaining (prioritized)

Tracked in the RC work below; see git history for what landed.

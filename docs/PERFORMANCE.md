# Performance

Pegoles must run smoothly on modest consumer hardware while eventually
hosting: UI + isolated VM + compositor + browser + runtime + local model
+ orchestration. Performance is architecture, not cleanup. Security
boundaries are never traded for speed anywhere below.

## Release candidate 0.1.0-rc.1, measured (M4 Pro 24 GB, macOS 26.5, 2026-09-26)

After the release hardening, on the sanitized v0.3 image and the bundled
runtime. Caveat: another GPU-heavy application (a flight simulator) was
running on the same Mac during these runs, so small regressions against
the earlier tables are within that noise; nothing was tuned for speed.

| Metric | Value | Source |
|---|---|---|
| VM start → guest ready | 3.4 s (3.1–3.4 s over 12 cycles) | `agent_e2e`, `release_soak` |
| First computer of an app session | + ≈ 4.5 s: the 3 GB image is re-hashed against its pin before the first clone (once per session) | `agent_e2e` prepare 8.2 s vs 3.6 s second session |
| Observe (capture + PNG) p50 / p95 | 136 / 156 ms over 1200 captures (141 / 161 ms in `agent_e2e`) | `release_soak` |
| Cancel → stopped | 1 ms (scripted wait); 2.2–3.0 s during local inference | `agent_e2e`, `local_e2e` |
| Guest runtime killed → ready again | 2.9 s | `agent_e2e` |
| Teardown (stop + destroy) | 0.9 s (scripted), 2.8 s with the worker | `agent_e2e`, `local_e2e` |
| Keyless task "create hello.txt … cat it" | 38.7 s and 53.0 s (2 passing runs) | `local_e2e` |
| Worker killed mid-task → task completed | 18–50 s | `local_e2e` |
| Model verify / load | 1.4 s / 1.2–1.5 s (MAI-UI-2B 6-bit) | `local_bench` |
| First token at 1024 px / 1440 px | 0.93 s / 2.0 s (unchanged by the sandbox) | `worker_probe` |
| Memory peak | worker 3.9–4.0 GB, VM 0.5 GB, app 0.05–0.12 GB | `local_e2e` |
| Guest memory over 2212 captures | flat (1256 → 1292 MiB available) | `capture_soak` |
| Host process over 12 lifecycles | footprint falls (380 → 126 MB), 4 descriptors throughout | `release_soak` |
| Computer image install (local archive) | 11.2 s to verify + unpack + hash 3 GB; 1.7 GB allocated (sparse) | `install_image` |
| Shipped runtime | 497 MB on disk, 59 native libraries | `build-runtime.sh` |
| DMG | 170 MB (app + runtime, no image or model) | `package-macos.sh` |

## Agent path, measured (M4 Pro, image v0.3, release build, 2026-09-26)

`cargo run --release -p pegoles-agent --example agent_e2e` (real VM, the
product orchestrator; helper copied beside the binary). Before = same
harness against the pre-session code paths where comparable.

| Metric | Before | After | Change |
|---|---|---|---|
| Cold boot → agent-ready (input + frame caps) | 15.1 s | 3.7 s | runtime waits for the Wayland socket before its first hello; idle capability re-probe 10 s → 2 s |
| Guest runtime killed → Ready again | 31.3 s | 3.2 s | host waits fail fast on channel loss (was: full 30 s capture timeout); reconnect backoff cap 30 s → 5 s |
| Observe (capture + PNG) p50 / p95 | — | 114 / 157 ms | RGB PNG, fast deflate, encoded with Core unlocked |
| Cancel → run stopped | — | 2–32 ms | cancel token checked in 50 ms wait slices and between steps |
| Scripted task (click, type 70 chars, Enter, wait 1.2 s, verify) | — | 2.3 s | |
| stop + destroy | — | 0.9 s | |
| Second session boot → ready | — | 3.1 s | |

Structural changes behind these: status reads never queue behind the
VM (cached status + a background pump that uses `try_lock`); waits never
hold Core; guest heartbeats no longer depend on the UI polling; bounded
helper→Core queue (1024 lines) replaces an unbounded channel.

## Pegoles Local, measured (M4 Pro 24 GB, 2026-09-26)

Model MAI-UI-2B 6-bit on MLX (worker process, sandboxed), real VM,
product runner. Details and all candidates:
`benchmarks/local-models/README.md`, `results.json`.

| Segment of one agent step | Time |
|---|---|
| Screen capture + PNG (guest → host) | 130–135 ms p50 |
| Image resize + decode in worker | ~15 ms |
| Prefill (first token), 1440×896 input, ~1 900 prompt tokens | 2.5 s p50 |
| Decode (~40–60 tokens incl. `<thinking>`, ~65 tok/s) | ~0.7 s |
| Parse + policy + executor + input | < 20 ms (typing: 8 ms/keystroke) |
| Settle wait before the next observation | 400 ms (fixed) |
| **Whole step** | **3.2 s p50 / 3.9 s p95** |
| Time to first action (task start) | 3.5 s p50 |
| Median task (22 benchmark tasks) | 11 s |

Prefill dominates, and it scales with visual tokens: 1024×640 input
gives first token 1.7 s / step 2.7 s at a small, noise-level accuracy
cost; 768×480 gives ~0.6 s / 1.0 s on a short prompt (not benchmarked
end to end). Native resolution stays the default for grounding
precision on small targets.

| Memory (phys_footprint) | Value |
|---|---|
| MLX worker, model loaded | 2.6 GB |
| MLX worker, steady during tasks | ~3.0 GB |
| MLX worker, peak (MLX cache capped at 256 MiB, default) | ~4.0 GB (3.9 GB in the keyless E2E) |
| MLX worker, peak without the cap | 5.0–5.5 GB |
| MLX worker after `unload` / after exit | 0.15 GB / 0 |
| VM process (1.5 GB guest), idle → during tasks | 0.49 → 0.55 GB |
| Pegoles runtime process without webview (harness) | 0.04–0.2 GB |
| **Pegoles total at peak (runtime + VM + model)** | **≈ 4.5–5 GB**, plus the desktop webview (not measured here) |

Model files: verify (full SHA-256, `ring`, HW-accelerated) 1.2 s per
2.2 GB, once per app session and in the background at startup; load
0.6–1.1 s warm. The worker stops after 10 minutes unused
(`IDLE_UNLOAD`) and gives back all of its memory.

Fixed on the way (measured regressions, not tuning):
- Guest runtime leaked a 5 MB memfd per screenshot: guest memory fell
  until the kernel OOM-killed the compositor after ~260 captures. Fixed
  (`guest/runtime/src/capture.rs`, `Drop` closes the pool); 400-capture
  soak now flat (`crates/pegoles-agent/examples/capture_soak.rs`), and
  the VM footprint stays ~0.55 GB instead of growing to 1.4 GB.
- Model verification used the portable SHA-256 (3.3 s per 1.8 GB);
  `ring` halves it.
- MLX kept freed Metal buffers: cache cap 256 MiB cuts ~0.9 GB of peak.

Not done (measured, deferred): prompt-prefix KV reuse (system prompt +
task are identical every step, ~500 tokens ≈ 0.8 s of prefill);
mlx-vlm's prefix cache is server-oriented and wants an on-disk tier the
worker sandbox forbids.

## Budgets (measured baseline, MacBook Pro Apple Silicon, 2026-09-21)

`scripts/bench.sh` reproduces these (`PEGOLES_REAL_GUEST_TEST=1` + RSS
sampling + disk usage; human-readable report, no hard gates):

| Metric | Baseline |
|---|---|
| create (artifacts + Apple validation) | ~80–350 ms |
| start → hypervisor Running | ~80–100 ms |
| VM start → GuestReady handshake | ~6–8 s (bimodal: guest boot variance; 9/11 green runs, see flakiness note) |
| Ping → Pong (vsock) | 0 ms |
| stop → Stopped | < 5 s (measured in e2e) |
| Helper RSS, running VM + guest | ~9–11 MiB steady (28 MiB transient max, 4 samples) |
| Helper CPU, 60 s idle VM | 0.0% avg |
| Guest RAM allocation | 1536 MiB (hypervisor-managed) |
| disk.raw logical / physical | 3.0 GB / ~1.4 GB (sparse) |
| disk.vhdx logical / physical | 3.0 GB / ~1.3 GB (dynamic) |
| Per-computer copy marginal cost | ~0 at creation (APFS clone-on-write; `du` overcounts shared blocks) |

Future phases compare against this file. Hosted CI never gates on
absolute numbers (hardware varies); `bench.sh` is the comparison tool.

Flakiness note (2026-09-21): the guest e2e failed twice in one window
(~11 s runs, logs unrecovered) then passed 9 consecutive runs across
two sessions. Suspected guest-boot/timing variance (Ready itself is
bimodal 6 s/8 s), not a logic defect — unit suite covers the session
deterministically. The e2e now logs computer id + guest state on every
failure path so the next occurrence is diagnosable.

## Phase 5 Eyes & Hands budgets

On-demand only — no streaming, no polling loops, no continuous capture:

| Metric | Budget / measured |
|---|---|
| Input round-trip (send + guest ack) | ≤5 s timeout; loopback ack ~0 ms (vsock ping baseline 0 ms) |
| Frame capture (1440×900 raw RGBA ≈5 MB, ~160 chunks) | ≤30 s timeout; loopback reassembly tested (200 KB in-ms) |
| FNV-1a dedup hash (5 MB) | few ms (GB/s class), on-demand only |
| Action burst brake | 20/s per computer (static caps: drag ≤10 s, text ≤4096 chars, wait ≤30 s, scroll ≤100 units) |
| Idle input cost | zero: no timers, no loops; rate limiter + pressed state are plain data |
| Audit log | 500 rows cap, content-free (verbs + lengths, never text) |
| Frame cache | 8 frames cap; events carry metadata only (no pixel spam) |

Hardware latency table (Apple Silicon M-series, `eyes_hands_smoke`, sealed v0.1 guest):

| Metric | Measured |
|---|---|
| create (fresh computer) | ~44 ms |
| start accepted | ~99 ms |
| guest ping (vsock round-trip) | 0 ms |
| input negotiation refusal (`input_execute` vs v0.1 guest) | <1 ms, structured `UnsupportedOperation` naming the missing cap |
| capture refusal (`GetFrame` vs v0.1 guest) | fast, same honest path (no 30 s hang) |
| stop | ~3.1 s |
| real move/click/type/scroll/drag + frame pixels | BLOCKED on guest runtime v2 deployment (sealed image rebuild — see Phase 5 report) |

## ResourceGovernor (`pegoles-computer/src/governor.rs`)

Single home for all resource heuristics. Inputs: total/avail RAM, CPU
cores, arch, power, pressure (Unknown until reliably detectable).
Output: `ComputerRecommendation { vcpus, memory_mb, profile, warnings }`.

Rules: ≤8 GB host → ~1 GB guest; ≤16 GB → ~1.5 GB; else ~2 GB; Eco
halves toward the 512 MB floor; Performance adds 50%; hard ceiling at
¼ host RAM; always ≥1 core left for the host; unknown host → safe
static default (2/1536). Exposed via Core `suggested_config()` +
Tauri `suggested_config`; `create` keeps explicit configs (advisory,
never silent override). Unit-tested for 8/16/32 GB profiles.

## Profiles

Eco (minimum footprint, battery-aware nudges) · Balanced (default/Auto)
· Performance (stronger hosts, still bounded) · Custom (validated +
clamped, never blind). No dozens of settings for normal users.

## Adaptive memory

- macOS: `VZVirtioTraditionalMemoryBalloonDeviceConfiguration` attached
  to every VM (hypervisor-managed ballooning + guest driver). Vz
  exposes no public host-driven target API, so no target calls exist —
  `BalloonPolicy::reclaim_target_mb()` computes targets (pure, tested:
  +25% headroom, 512 MB floor, assigned cap, no-op without evidence)
  for future enforcement points.
- Windows (designed): Hyper-V Dynamic Memory converges toward the same
  policy outputs via HCS modify calls when the backend lands.
- No `DynamicGuestMemory` trait with fake impls: the policy struct IS
  the generic abstraction; enforcement is per-adapter.

## CPU budget

Default 1–4 vCPUs (governor), Custom clamped to cores−1 (max 8). Never
all host cores; background work prefers efficiency. Verified by
low-end profile tests, not by benchmarking real hardware.

## Event-driven, no polling loops

There are no 50 ms-style loops anywhere: VM truth arrives via helper
responses + delegate/callback events; guest truth via vsock frames;
UI via EventBus + a 2 s status poll ONLY while a VM runs with an
unsettled guest (handshake window), silent otherwise. Heartbeat is
10 s Ping/Pong with no UI/log noise (`poll_guest` is non-blocking;
`pump_stream` blocks in its own thread, never the UI).

## Idle lifecycle (modeled, not automated)

`IdleState::{Active,Idle,Paused,Stopped}` + `IdlePolicy` (all thresholds
`None` = manual). Rationale (§46–47): GuestReady in ~6 s makes
pause/stop-and-resume cheap, so future schedulers may idle-reclaim;
today nothing acts automatically, and background tasks will be able to
opt out. Any future auto-policy must be configurable.

## Guest minimalism (audited 2026-09-21, derived v0.1)

Enabled: e2scrub_reap, grub-common, pegoles-guest-runtime, remote-fs,
ssh, systemd-networkd, unattended-upgrades (+ apt-daily/man-db timers).
Candidates for v0.2 removal (clearly unnecessary offline, safe):
`ssh.service` (no inbound-SSH use; control plane is vsock) and
unattended-upgrades/apt timers (network-dependent, useless offline).
NOT removed in 3.6: the sealed v0.1 image stays byte-stable for e2e;
removal + re-verification belongs to the next image build. Never
GNOME/KDE/XFCE; no mass-disable scripts, ever.

## Guest runtime CPU

Event-driven by construction (blocking vsock read, no timers except
capped reconnect backoff + answering host pings). Host-measured idle:
helper 0.0% CPU over 60 s with a running VM. Guest-internal CPU is not
directly observable from the host; the runtime does no periodic work
of its own.

## Log control

No heartbeat/activity noise by design (Ping/Pong never emit UI events
or logs). `logs/serial.log`: fresh file per creation; enforced cap
(8 MiB → keep last 1 MiB, UTF-8-safe cut) applied only while the VM is
stopped, in `backend_start`/`backend_stop` — never truncating a file
the helper holds open. Helper stderr is discarded (null); diagnostics
travel as typed JSONL errors.

## Storage efficiency

- macOS RAW: sparse (3.0 GB logical → ~1.4 GB physical).
- Per-computer copies: APFS clone-on-write (~0 marginal at creation).
- Windows VHDX: dynamic subformat (~1.3 GB for 3 GB logical).
- Nominal disk sizes stay 20 GB (config); physical growth is on demand.
- VHDX container bytes embed creation metadata (GUID/timestamps), so
  re-conversion is content-identical but not byte-identical: verify by
  converting + booting, never by cross-build byte comparison.

## One VM per active need

Future multi-agent work distinguishes Agent from Computer: agents may
share a computer when policy allows; isolation stays available where
required. No running VM per inactive agent (fast ~6 s resume makes
pause/stop viable on laptops).

## UI performance tiers (Phase 4 preparation)

Tiers: Full Effects / Reduced Effects (less blur/glow, shorter trails,
fewer ambient animations, lower sampling cost) — functionality never
depends on effects. Global CSS already honors `prefers-reduced-motion`
(see `globals.css`); ambient animations must sleep when unfocused /
minimized / invisible. No full-screen 60 fps idle loops, ever.

## Model runtime

Implemented (see "Pegoles Local, measured" above and
`docs/MODEL_ARCHITECTURE.md`): lazy load on first task, model kept
resident between tasks, idle unload after 10 minutes, explicit unload,
quantized models, MLX cache cap. Small/large routing and hybrid cloud
routing are designed (a planner composed of planners) but not built.

## Memory pressure architecture (future sequence)

Unknown by default (no reliable source wired yet). Designed escalation:
balloon target down → pause non-critical VM → unload inactive model →
notify user. Never silently kill active work; never crash the host.
Detection points (documented, unwired): macOS `memory_pressure` events
via the helper, Windows memory events via HCS properties.

## Battery

`PowerSource::{AC,Battery,Unknown}`; detection returns Unknown (no new
daemons, no fragile spawns). Eco + Battery already nudges minimum
footprint in the governor. Future battery behavior (fewer effects,
less concurrency) keys off this enum when a reliable source lands.

## Low-end validation

`governor.rs` tests encode 8 GB/4-core, 16 GB/8-core, 32 GB/12-core
decision tables (logic only — they do not benchmark hardware).

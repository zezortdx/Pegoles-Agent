# Performance (Phase 3.6 foundation)

Pegoles must run smoothly on modest consumer hardware while eventually
hosting: UI + isolated VM + compositor + browser + runtime + local model
+ orchestration. Performance is architecture, not cleanup. Security
boundaries are never traded for speed anywhere below.

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

## Model-runtime contract (Phase 6 preparation, no implementation)

The future Local Model Router must support lazy loading, idle eviction,
unloading, quantized models, and small/large routing (simple→small,
visual→vision, hard→large), then release resources. Nothing in 3.6
assumes a permanently resident model; the governor's ¼-RAM ceiling
already reserves headroom for it.

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

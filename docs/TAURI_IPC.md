# Tauri IPC

Frontend (`apps/desktop/src`) never owns state. It calls these commands
and subscribes to event streams; Pegoles Core decides everything
(viewport state, display lifecycle, control ownership, readiness).

## Threading (Phase 4)

The macOS VM runs in-process and Virtualization.framework needs the main
queue, while Tauri 2 runs *sync* commands on the main thread. Therefore
every command that touches Core is an `async fn` that runs its work on
Tauri's blocking pool (`with_state` in `src-tauri/src/commands.rs`): the
main thread never waits for a VM reply or for the `AppState` lock, and
the lock is never held across an `.await`. Only lock-free
`get_host_capabilities` stays sync. From JavaScript nothing changes:
every command returns a Promise; errors reject with a `string`.

Argument names: Tauri maps top-level JS keys from camelCase
(`prefersReducedMotion`) to Rust snake_case parameters. Nested objects
(e.g. `geometry`) keep the snake_case field names shown below.

## Commands (`src-tauri/src/commands.rs`)

| Command | Args | Returns |
|---|---|---|
| `get_status` | — | `StatusPayload`. Pumps Core when it is free; while a long operation holds Core it returns the cached status (a background thread pumps every 500 ms) |
| `pump` | — | `{ events: AgentEvent[], status: StatusPayload }` — same work as `get_status` plus the events THIS pump published |
| `create_computer` | — | `{ info }` — resumes the computer built from the current image, or clones a fresh one |
| `start_computer` / `resume_computer` | — | `{ info }` |
| `pause_computer` / `stop_computer` | — | `{ info }` — cancels any agent run first |
| `reset_computer` | — | `{ info }` — fresh disk + EFI store from the sealed image; refused during a run |
| `destroy_computer` | — | `{ info: null }` — removes the computer; refused during a run |
| `list_events` | — | `AgentEvent[]` (session-only live history, oldest first, capped at 200) |
| `read_boot_log` | — | `{ available, total_lines, tail }` |
| `guest_info` / `guest_ping` | — | diagnostics (not used by the product UI) |
| `get_host_capabilities`, `suggested_config`, `suggested_effects`, `accessibility_display` | … | host facts |
| `display_set_geometry` / `display_detach` | … | native view (stub on macOS today) |
| `take_control` / `return_control` | — | `ComputerView`; take_control cancels any agent run first |
| `create_task` | `title` | `AgentTask` (pending) |
| `list_tasks` | — | `AgentTask[]` including retained previous tasks; recovered open tasks are cancelled |
| `get_history_status` | — | `{ read_only: boolean, previous_task_ids: TaskId[] }`; true means disk history could not load/save and new changes are not saved |
| `get_task_history` | `taskId` (UUID) | `TaskHistory \| null`; fixed identifier lookup, no path/URL/action arguments |
| `run_task` | `taskId` | starts the agent on a pending task (one run at a time) |
| `cancel_task` | `taskId` | stops the run, or cancels a pending task |
| `get_model_settings` / `set_api_key` / `clear_api_key` / `set_model_settings` | … (Anthropic, optional) | `{ configured, key_source, model, effort, models, efforts }` — the key is never returned |
| `get_intelligence` | — | `{ provider: "local"\|"anthropic", local_model, local: { runtime_ready, runtime_problem, loaded_model, worker_footprint_bytes, default_model, models[], install, chip, memory_bytes, apple_silicon }, anthropic: ModelSettings }` — no secrets |
| `set_provider` | `provider`, `localModel?` | `Intelligence` (persists `settings.json`) |
| `install_local_model` | `model?` (default: chosen) | `Intelligence`; download → verify → atomic install in the background, progress on `pegoles://model-install`; only catalog-offered models |
| `cancel_local_model_install` | — | `Intelligence` (partial download kept, resumable) |
| `remove_local_model` | `model` (catalog id) | `Intelligence`; refused while a task runs |
| `cancel_agent_input` | — | stops any agent run and releases held input |
| `capture_screen` | — | `{ meta, png_base64 }` (guest framebuffer only; encoded with Core unlocked) |
| `input_status` / `input_audit` | … | input plane facts; content-free audit rows |
| `get_image_status` | — | status of the sealed Pegoles image |
| debug builds only: `prepare_image`, `execute_action`, `run_input_script`, `demo_script_steps` | … | Design Lab / image-builder helpers; absent from release builds |

### `StatusPayload`

```ts
type StatusPayload = {
  core: "running";
  model: "not_configured";
  backend: "mock" | "real" | "windows-hcs";
  computer_created: boolean;
  computer_state: ComputerState | null;
  computer_id: string | null;
  image_status: "missing" | "downloading" | "ready" | "invalid";
  spec_os: "Debian 13";
  spec_arch: "arm64" | "amd64";
  spec_vcpus: number;
  spec_ram_mb: number;
  guest_state: "unavailable" | "waiting" | "connecting" | "ready"
             | "disconnected" | "incompatible" | "error";
  guest_ready_ms: number | null;
  display_setup_error: string | null; // display adapter failed to install at startup
} & ComputerView; // flattened (same top-level object)

type ComputerState = "stopped" | "starting" | "running" | "paused" | "stopping" | "error";
```

### `ComputerView` (Phase 4 computer surface)

```ts
type ComputerView = {
  viewport_state: ViewportState;
  viewport_issue: ViewportIssue | null;
  display_available: boolean;      // framebuffer configured AND this process can show it
  display_config: { profile: DisplayProfile; width_px: number; height_px: number } | null; // null = headless
  display_backend: "unavailable" | "mac_virtual_machine" | "windows_hyper_v" | "test";
  display_attached: boolean;       // native view CONFIRMED attached
  graphical_session: GraphicalSession;
  control_owner: "none" | "user" | "agent"; // "agent" = a deterministic Phase 5 sequence owns input (policy-checked)
  display_ready_ms: number | null; // VM start -> DisplayReady (this boot)
  display_error: string | null;    // last display failure/diagnostic
};

type ViewportState =
  | "off" | "preparing" | "starting" | "guest_connecting" | "display_starting"
  | "ready" | "paused" | "agent_active" | "user_controlled" | "error";

type ViewportIssue =
  | "computer_error"                 // VM in Error
  | "guest_incompatible"             // guest protocol version mismatch
  | "guest_error"                    // handshake timeout / protocol violation
  | "graphical_session_failed"       // compositor failed/exited (guest report)
  | "graphical_session_unavailable"  // display configured but guest reports no session
  | "display_failed"                 // native view failed; send a new geometry to retry
  | "display_backend_unavailable";   // NOT an error: viewport "ready" but headless (no adapter)

type DisplayProfile = "desktop_large" | "desktop_compact" | "mobile_remote"
                    | "low_bandwidth_remote" | "custom";

type GraphicalSession = {
  state: "unavailable" | "starting" | "ready" | "failed";
  reported: boolean;          // guest reported at least once this boot
  compositor: string | null;  // e.g. "weston"
  width_px: number | null;
  height_px: number | null;
  detail: string | null;      // guest diagnostic (bounded)
  since_ms: number | null;    // VM start -> report that entered `state`
  ready_in_ms: number | null; // VM start -> first Ready this boot
};
```

`viewport_state` is derived by a pure Core function
(`pegoles_core::viewport::derive_viewport`) from: image preparation,
computer state, guest handshake state, graphical-session report, display
attachment, control owner. Rules, in order:

1. no computer / `stopped` / `stopping` → `off` (`preparing` while the
   base image is being prepared); `starting` → `starting`; `paused` →
   `paused`; VM `error` → `error` (`computer_error`).
2. running, guest not Ready → `guest_connecting` (`incompatible` /
   `error` → `error`).
3. guest Ready and no display (`display_available: false`) → `ready`
   (headless; issue `display_backend_unavailable` only when a framebuffer
   exists but this process has no adapter). Never an invented state.
4. display: compositor `failed` → `error`; reported `unavailable` →
   `error`; attach failure → `error` (`display_failed`); compositor not
   Ready or view not attached → `display_starting`; else `ready` /
   `user_controlled` (control `user`).

Backends without a guest control plane (Mock, dev only) are `ready` as
soon as they run.

### Display geometry

```ts
type DisplayGeometry = {
  rect: { x: number; y: number; width: number; height: number }; // CSS px, webview top-left
  visible: boolean;    // false = hide (modal over the slot, navigated away); not destroyed
  animate_ms?: number; // native animation to the new rect, 0..=1000 (default 0)
};
type GeometryOutcome = "attached" | "updated" | "unchanged" | "deferred";
```

- Send ONLY on layout change / window resize / mode change — never per
  animation frame. For layout transitions send the FINAL rect once with
  `animate_ms`; the native side animates.
- Untrusted input: Core rejects non-finite, negative, fully-outside and
  over-long-animation geometry (`"invalid display geometry: NotFinite" |
  "Negative" | "Empty" | "AnimationTooLong"`), clamps to the webview
  bounds and snaps to device pixels. Bounds + scale are read from the
  HOST window (never from React).
- `unchanged`: jitter < 0.5 CSS px (no native call). `deferred`: stored,
  not applied (no computer, not running, headless, or no adapter); Core
  attaches automatically when the computer can show a framebuffer (also
  across stop → start: the slot is remembered until `display_detach`,
  destroy, or a native-initiated detach).
- Keep the slot at the `display_config` aspect ratio; nothing in the DOM
  may render over the rect (the native view sits above the webview).

### Control

- `take_control`: when `viewport_state == "ready"` and
  `display_available` (error string starts with `control unavailable:`),
  OR as takeover from `agent` (cancels the agent sequence and releases
  pressed state first — never simultaneous control).
  Routes the HUMAN's keyboard/pointer into the isolated computer.
- `return_control`: always allowed, idempotent.
- Automatic return to `none` (with a `control_ownership_changed` event)
  on pause, stop, reset, destroy, VM error, display detach/failure, and
  whenever the display stops being ready (e.g. compositor failure, guest
  runtime disconnect). Resume never re-grants control.
- The native "Return to Pegoles" pill / escape shortcut arrive as a
  display fact; Core honors them on the next `pump`.
- `agent` is reserved (future policy-checked structured actions). No
  Phase 4 API can set it (guard-tested).

### Effects

```ts
type EffectsRecommendation = {
  tier: "full" | "reduced" | "minimal";
  reasons: Array<
    | "host_unknown" | "very_low_memory" | "very_few_cores"
    | "memory_pressure_critical" | "low_memory" | "few_cores"
    | "eco_profile" | "on_battery" | "prefers_reduced_motion" | "capable_host">;
};
```

Rules (governor): unknown host → `reduced`; ≤ 4 GB RAM or ≤ 2 cores or
critical memory pressure → `minimal`; ≤ 8 GB or ≤ 4 cores → `reduced`;
Eco profile and battery (macOS: IOKit power-source estimate; elsewhere
unknown) cap at `reduced`; otherwise `full`. `prefers_reduced_motion` is
echoed as a reason and NEVER changes the tier — motion is an
accessibility preference the UI honors independently.

### Tasks

```ts
type AgentTask = {
  id: string; // uuid
  title: string;
  status: "pending" | "running" | "waiting_for_approval" | "completed" | "failed" | "cancelled";
  created_at: string; // RFC 3339
  updated_at: string;
};
```

`create_task` trims the title and requires 1..=500 characters, single
line, no control characters (`"invalid task title: …"`). Phase 4 has no
model: nothing runs tasks, they stay `pending` — say so in the UI.

## Events

- Live: `listen("pegoles://event", …)` — every `AgentEvent` published on
  Core's `EventBus` (bridged in `lib.rs`).
- History: `list_events()` — drained from a dedicated bus subscriber
  after every command, so history carries exactly the live events (same
  timestamps, no heartbeat noise).
- Image progress: `listen("pegoles://image-progress", …)`.
- Display wake: `listen("pegoles://display-activity", …)` (emitted by the
  native display layer) → call `pump()`; never poll per frame.
- Polling guidance: `get_status` every ~2 s only while `viewport_state` is
  `starting`, `guest_connecting` or `display_starting` (guest reports
  arrive over vsock and are processed by pumps); silent otherwise.

Wire format: `AgentEvent` serialized with serde, tagged by `type`
(snake_case). Phase 4 variants and when Core emits them:

| `type` | Fields | Emitted when |
|---|---|---|
| `graphical_session_ready` | `computer_id, compositor, width_px, height_px, ready_in_ms, at` | guest reports its compositor Ready (real in-guest round-trip). Size as reported, else the configured scanout. `ready_in_ms` from VM start. Again after a Failed → Ready recovery. |
| `graphical_session_failed` | `computer_id, message, at` | guest reports compositor failed/exited |
| `display_attached` | `computer_id, at` | native view CONFIRMED attached (native fact, FIFO-matched to Core's latest attach) |
| `display_ready` | `computer_id, ready_in_ms, at` | graphical session Ready AND view attached — exactly once per boot |
| `display_detached` | `computer_id, reason, at` | a confirmed view went away. `reason`: `computer_stopped`, `computer_reset`, `computer_destroyed`, `computer_error`, `explicit`, `display_replaced`, `display_failed: …`, or the native reason for native-initiated detaches |
| `control_ownership_changed` | `computer_id, from, to, at` | take / return / every automatic return; an agent run holds `agent` for its whole duration |
| `agent_message` | `task_id, kind (progress/summary/error), text, at` | the agent's narration: notes between actions, the final summary, or why the run stopped. Untrusted model text: render as plain text only |

Guest lifecycle events (`guest_runtime_*`, `computer_*`, `task_*`) are
unchanged. Starting/Unavailable graphical reports only move
`viewport_state` (no event). Heartbeats never produce events.

`pegoles://model-install` carries `{ model, phase: downloading|verifying|finalizing|ready|failed|cancelled, done_bytes, total_bytes, error, error_kind: disk_space|network|corrupted|other }`, at most ~4/s while downloading and on every phase change.

## State ownership

`src-tauri/src/state.rs` (`Mutex<AppState>`) owns `ComputerRegistry`
(computer + display adapter + control owner) `+ TaskManager + event_log`.
React holds only rendered copies.

## Local appearance preferences (Agent 2.0)

`get_preferences()` returns `Preferences`; `set_preferences({ preferences })`
returns the validated, normalized record only after durable replacement. Both
are release commands granted only to the local main window. They run on the
blocking pool under a store-specific mutex, independently of the VM/Core lock.
There is no path, URL, CSS, SVG, provider, authority, or planner argument.

```ts
type Preferences = {
  schema_version: 1;
  avatar: {
    shape: "circle" | "pebble" | "squircle" | "capsule" | "triangle" | "hexagon" | "cloud" | "droplet";
    color: "silver" | "graphite" | "pearl";
    expression: "neutral" | "attentive" | "surprised" | "excited" | "happy" | "laughing" | "angry" | "sad" | "afraid" | "suspicious" | "confused" | "curious" | "proud" | "shy" | "bored" | "sleepy";
    name: string;
  };
  locale: "en" | "pt-BR";
};
```

Core rejects unknown fields at both object levels and validates every enum and
version. Names are trimmed, 1–32 Unicode scalar values, without control
characters or bidi controls (including at the edges before trimming). Defaults:
Pegoles, circle, silver, attentive, en. Only a missing file means defaults;
corrupt, oversized, unsupported and unreadable files reject the command.

The sole destination is `<pegoles_data_dir>/preferences/profile.json`. Reads
are capped at 4096 bytes, regular files only. Directory components and file
symlinks are refused; Unix leaf opens additionally use `O_NOFOLLOW` and
`O_NONBLOCK`. The directory is private (0700), temporary files are created
exclusively (0600), synced, and atomically renamed; the directory is synced on
Unix. Failed writes reject, clean the temporary file and never invent success.
Same-user host filesystem races are outside the existing threat model; Windows
ACL/reparse-point behavior still needs platform verification. The webview has
no filesystem permission. Nothing is added to `ComputerAction` or model context.

The React `PreferencesProvider` owns only a rendered copy. It loads once, offers
retry on failure and applies saved appearance/locale after Core confirms.
`usePreferences()` exposes the record and `save()` for subsequent product work;
`useLocale()`, `currentLocale()` and `t()` expose the typed static copy catalog.
Language changes preserve task IDs and drafts. User, guest and model text is
interpolated/rendered as original text, never used as a translation key or HTML.
The named preview is a UI simulation; it never changes Core state. Saved
expression decorates resting poses; work, blocked, offline and error states
retain their real-state precedence and existing motion guards.


## Bounded task history and result artifacts (Agent 2.0)

Core `history.rs` projects task metadata and status transitions explicitly;
it never serializes `AgentEvent` or `ActionRequest`. AppState syncs this
projection after commands and agent calls on blocking workers. Production
opens fixed `<pegoles_data_dir>/history/tasks.json`; `with_registry` is fully
in-memory and tests inject only temporary stores with `with_history_store`.

```ts
type TaskHistory = {
  task: AgentTask;
  audit: { status: TaskStatus; at: string }[]; // up to 32; no diagnostic text
  artifact: { task_id: TaskId; data:
    | { kind: "text"; text: string }
    | { kind: "table"; columns: string[]; rows: string[][] }
  } | null;
  recovered: boolean;
  output_blocked: boolean; // privacy marker only, never approval authority
};
```

Only the existing final `Summary` message can become the one result artifact.
The UI labels its source as an agent summary and renders plain text/HTML table
cells; this is neither a guest-file extraction nor a host-file viewer.
No artifact contains a host path to open, remote URL to load, HTML renderer,
Markdown interpreter or executor permission. Unknown task IDs return null;
malformed UUIDs reject before the lookup. The main-window ACL grants only
these reads; there is no webview/model history write command.

The canonical JSON cap is 512 KiB. One fixed ≤512 KiB stage bounds aggregate
snapshot bytes to 1 MiB during/interrupted replacement, plus an empty permanent
OS-lock file. Reads/writes hold the same nonblocking OS lock before bounded,
no-follow recovery of reserved abandoned stages; busy/invalid/unrecoverable
stores report read-only, never cleanup an active writer. Exact item/scan limits,
taint suppression and recovery semantics are in [PRIVACY.md](PRIVACY.md). Raw live events are not replayed on restart.

### Human approval (session only)

- `get_approvals()` returns `{ ask_before_input: boolean, pending: PendingApproval[] }`.
  At most eight pending entries: `{ id, task_id, computer_id, summary, expires_at }`.
  `id` is the opaque UUID of the original action; summaries contain only a
  Core-generated VM-input verb. No typed payload, key names, paths or arguments.
- `set_input_approval({ enabled: boolean })` requires native confirmation for
  either direction. The Core-owned flag defaults to false and resets to false
  on app restart. It is absent from appearance preferences and model tools.
- `resolve_approval({ id: UUID, decision: "approve_once" | "deny" })` accepts no
  replacement action or blanket grant. Approve once opens the existing native
  ConsentGate with Core's safe summary, outside the AppState lock. Native refusal
  denies. Deny needs no native prompt. The command rechecks the task and exact
  pending record after the native response; a stale response cannot dispatch.

Only the local main window gets these commands in the release ACL. The typed
planner/guest vocabularies have no approval or policy-changing action. Core's
120-second `Instant` deadline is authoritative; `expires_at` is display-only.
The runner publishes a bounded Core-owned denial/expiry/interruption reason
in the live activity feed (not persisted). Current policy, control generation/owner, computer identity, cancellation,
input shape and backend capability checks still apply on dispatch. One grant
covers only the stored action; long typing slices that stored payload, with
whole-payload checks and cancellation between slices. Transport primitive IDs
include the completed character offset so slices never reuse an acknowledgement ID. No approval survives
restart, stop, reset, task completion or control changes. A denial/expiry ends
the current run without inviting the planner to keep requesting approval.

Pending requests/grants are in memory only. Cleared payloads leave at most eight
content-free terminal rows (IDs, reason, successful typed-prefix counts), consumed
by the waiting runner. History keeps its existing status-only audit and privacy
taint behavior. Raw input and approval grants never enter its persisted schema.

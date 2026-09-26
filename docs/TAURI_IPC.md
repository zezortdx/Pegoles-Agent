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
| `list_events` | — | `AgentEvent[]` (history, oldest first, capped at 200) |
| `read_boot_log` | — | `{ available, total_lines, tail }` |
| `guest_info` / `guest_ping` | — | diagnostics (not used by the product UI) |
| `get_host_capabilities`, `suggested_config`, `suggested_effects`, `accessibility_display` | … | host facts |
| `display_set_geometry` / `display_detach` | … | native view (stub on macOS today) |
| `take_control` / `return_control` | — | `ComputerView`; take_control cancels any agent run first |
| `create_task` | `title` | `AgentTask` (pending) |
| `list_tasks` | — | `AgentTask[]` |
| `run_task` | `taskId` | starts the agent on a pending task (one run at a time) |
| `cancel_task` | `taskId` | stops the run, or cancels a pending task |
| `get_model_settings` / `set_api_key` / `clear_api_key` / `set_model_settings` | … | `{ configured, key_source, model, effort, models, efforts }` — the key is never returned |
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

## State ownership

`src-tauri/src/state.rs` (`Mutex<AppState>`) owns `ComputerRegistry`
(computer + display adapter + control owner) `+ TaskManager + event_log`.
React holds only rendered copies.

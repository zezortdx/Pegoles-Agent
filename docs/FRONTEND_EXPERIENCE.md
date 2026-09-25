# Pegoles frontend experience

Design for delegation, not supervision. Pegoles is a persistent agent with a
computer of its own; the interface exists so a person can hand work over,
glance at it, step in when asked, and leave again.

This document is the plan of record for the 2026-09-24 redesign. Token values
and component rules live in [DESIGN_SYSTEM.md](DESIGN_SYSTEM.md) §0.

> **Superseded for the shell (2026-09-26).** The desktop shell was rebuilt
> around a Codex-grade structure (sidebar · work · contextual Computer
> panel · one bottom work bar). The current shell, screens, Computer levels
> and motion are described in DESIGN_SYSTEM.md §0. The honest-state rules
> and the Core signal mapping below still hold; the screen descriptions in
> §9–§10 are history.

## 1. Current state (Phase 0 audit, 24 September 2026)

Baseline before the redesign: desktop typecheck, lint, 11 test files / 45
tests and production build green (entry JS 125 kB gz, CSS 12.6 kB gz);
`@pegoles/ui` typecheck and 21 files / 175 tests green.

What the product can really do today, from the code:

| Capability | Reality |
|---|---|
| Tasks | `create_task` / `list_tasks`. In memory, single line, ≤ 500 chars. Every task stays `pending`: no model runner exists (`StatusPayload.model = "not_configured"`). |
| Agent execution | None in production. `execute_action` / `run_input_script` exist and emit real `action_*` events, but only the dev lab calls them. |
| Approvals | Policy emits `approval_requested` and `needs_approval` outcomes. There is **no** approve/reject command. |
| Computer | Prepare image → create → start → guest boot events → ready; pause / resume / stop; `take_control` / `return_control` with `ControlOwner = none \| agent \| user`. |
| Computer screen | Native `VZVirtualMachineView` over the WebView, positioned from a DOM slot. The embed is a stub (`native_display.rs`), so `display_available` is false today. `capture_screen` exists but logs a `frame_observed` event per call, so it is not a preview stream. |
| Policy | Deterministic: host paths blocked, credentials never typed, web access and writes outside the workspace ask first, dangerous shell blocked. Typed text never stored in the audit log. |

Problems found: two unrelated token systems (28 shell vars, 193 package vars,
nine blues, 17 radii, glow shadows); the same state shown three or four times
(task title ×4, pending ×4, computer state ×4, control owner ×3); technical
details and raw backend errors in the primary hierarchy; an "off" computer
taking half the screen; suggestions that invite work nothing can do;
"Task saved" copy for tasks that vanish on quit; any IPC error turning the
global presence to `error`; a default window (880×640) below the layout's
design width.

## 2. Principles

1. **Presence is the interface.** One Pegoles object carries state. No spinner
   + badge + status text for the same fact.
2. **Their computer, not yours.** Computer has three levels: status → preview →
   takeover. It never occupies the screen by default.
3. **Reassurance first, detail on demand.** Motion and one line of current
   activity by default; recent actions on hover/click; diagnostics in Settings.
4. **Shape of information.** Files look like files, approvals like approvals,
   outcomes like outcomes. One mixed timeline per task.
5. **Honest state only.** Every animation and line of copy maps to a real
   Core signal or a real user action. Nothing is invented to look busy. No
   hidden reasoning is shown or fabricated.
6. **Quiet → aware → alive → calm.** The interface is calmest when nothing is
   happening.

## 3. Target architecture

```
Core snapshots + events (lib/tauri.ts, unchanged IPC)
        │
        ▼
state/useCore.ts ── polling (2.5 s, visible only) + pegoles:// events
        │            scoped actions: errors routed to task / computer / general
        ▼
state/agentState.ts      VisualStateAdapter (pure):
state/transcript.ts        task + events + status → PresenceMode, current
state/computerModel.ts     activity, recent actions, transcript items,
state/errors.ts            computer phase + copy, humanized errors
        │
        ├──► presence/      <PegolesPresence mode size look pulse />
        │                    SVG first paint + fallback; lazy WebGL2 SDF
        │                    renderer (one shared canvas); thought contours
        ├──► thought/       SVG overlay: ephemeral Presence → Computer thread
        └──► DOM product UI shell/ home/ composer/ task/ artifacts/
                             computer/ activity/ settings/
```

The visual layer never owns business logic: it receives a `PresenceMode`,
an optional gaze target and a pulse counter that increments on real events.

### Presence modes and their sources

| Mode | Real source |
|---|---|
| `offline` | Core not connected / browser preview |
| `idle` | nothing pending attention |
| `attentive` | composer focused or typing |
| `acknowledging` | user submitted a task (until Core answers, ≤ 900 ms after) |
| `thinking` | task `running`, no action in flight |
| `planning` | reserved: no Core signal yet (never produced) |
| `working` | `action_started` in flight (Files / Shell / Web) |
| `using-computer` | computer action in flight, or viewport `agent_active` |
| `waiting` | the user has control of the computer |
| `needs-user` | `waiting_for_approval`, `approval_requested`, `needs_approval` |
| `blocked` | pending with no model connected; action denied by policy |
| `done` | task `completed` (one settle, then calm) |
| `error` | task `failed`, action failed, computer failed in its own context |

## 4. Surfaces

- **Home** — Presence, "What now?", composer. Nothing else.
- **Task** — transcript of mixed items: your request, Pegoles' live activity
  (Presence + one line, expandable), action outcomes, files, approvals,
  results. No follow-up composer: Core has no message-to-task command, so a
  composer here would be a dead control.
- **Sidebar** — mark + New task, recent work (second line only for tasks that
  need a glance), Computer / Activity / Settings. Search appears past eight
  tasks. No health rows.
- **Computer** — status chip (top right) → pinned preview (≈34 %) → takeover.
  Preview and takeover are one mounted component so the native slot never
  detaches mid-handoff. Off is a status, not a monitor. Ownership is always
  explicit: "Pegoles has control" / "You have control".
- **Activity** — chronological stream by day, human sentences, technical
  details folded.
- **Settings** — General, Pegoles, Computer, Security (the real policy, read
  only), Appearance (motion quality Auto / Full / Reduced), Developer.
- **Errors** — people get a sentence and a next step; engineers get Details.

## 5. Motion

Tokens: micro 80–160 ms, layout 240–320 ms, agent 420–700 ms, scene
650–900 ms; easings `out`, `move`, `light`; no bounce anywhere. Motion (the
library) handles shared layout (home prompt → first transcript item) and
presence; CSS handles hover/focus/state. Continuous animation runs only while
the ambient gate says the window is visible, focused and recently used.
Reduced motion keeps every state legible through light, opacity and text.

## 6. Performance budget

Entry JS ≤ 150 kB gz; the WebGL presence chunk loads after first paint (target
≤ 10 kB gz, no three.js). One WebGL context for the app; canvas sized to the
presence (never full screen), DPR ≤ 2 (1.5 while the computer is in use);
0 fps when hidden, blurred, idle or settled; ≤ 30 fps idle life, ≤ 60 fps while
changing state. Auto quality steps DPR → field detail → SVG on slow frames.
Nothing draws over the native computer slot.

## 7. Migration risks

- Native view tracks DOM geometry with a 100 ms debounce: never transform an
  ancestor of the computer slot; keep preview/takeover in one instance.
- Tests assert old copy and page structure: rewrite them against the new
  behaviour, not the old markup.
- `@pegoles/ui` glass components remain for the design lab only; the shell no
  longer depends on their styling.
- Browser previews stay disconnected; dev fixtures live only behind
  `import.meta.env.DEV` and are checked out of the production bundle.

## 8. Evolution: structure, material, life (25 September 2026)

The first pass was clean but read as unfinished: a flat sidebar, a black
void under Home, a static-looking Presence, a settings-like Computer panel.
This pass keeps that system and adds structure (Codex-like), material
(Apple Liquid Glass as a functional layer) and life (Pegoles), without
dashboards, neon or invented activity.

### Layout

`sidebar (inset glass, 268–276 px ↔ 64 px rail) │ workspace │ inspector
(optional, clamp(340px, 29vw, 420px))`. The title bar (52 px) holds the
task's state and elapsed time, reveals the task title once the request
scrolls away, and the Computer control; the inspector's header shares
that row. Narrow windows (< 1100 px) keep the rail and open the full
sidebar as a drawer.

### Depth

L0 window and ambient light (grain, low-frequency luminance, the light
Pegoles casts, a cool highlight near its computer while in use) · L1 matte
content · L2 navigation and inspector chrome (glass) · L3 floating controls,
palette, toast (`.glass-float`). Glass never sits on the transcript,
artifacts, the computer's screen or settings rows.

### Native material decision

The window stays **opaque**; there is no `macOSPrivateApi`.

- Tauri's window effects need a transparent window, and on macOS a
  transparent WKWebView needs the private `drawsBackground` key
  (`macos-private-api`). That blocks the App Store and can break on WebKit
  updates.
- Tauri issue #15471 measured about 8× GPU power for transparent windows on
  macOS 26.5.
- Glass is therefore CSS: tint, backdrop blur and saturation, sheen, rim
  and specular edge, over the L0 ambient field that gives it something to
  diffuse.
- Native Liquid Glass (`NSGlassEffectView`, public API) is reserved for
  controls that must float over the native VM view, once that view exists.
- WKWebView ignores `prefers-reduced-transparency`. Rust therefore reports
  `NSWorkspace` Reduce Transparency and Increase Contrast
  (`accessibility_display`), and the shell mirrors them onto `<html>`.
  Glass turns solid when either is on.
- If real sidebar vibrancy is ever wanted, the recipe (a flagged Developer ID
  build: private API plus a glass pane under the webview) is in the research
  notes; it is not shipped.

### Presence renderer decision

Keep and upgrade the custom WebGL2 SDF raymarcher (≈ 9 kB gz) instead of
adding three.js/R3F (133–189 kB gz). `MeshPhysicalMaterial` transmission
can't refract DOM behind a canvas anyway, and the SDF stays exactly
faithful to the brand geometry. The upgrade adds a perspective camera, slow
sway, an analytic studio environment, fresnel, edge scattering, an internally
lit face and a 60 fps clamp.

### Reasoning

The Thought Ribbon is SVG + WAAPI beside the mini Presence in the live turn.
It draws one strand per real `action_id`: it sprouts on request, carries a
signal while running, holds amber on approval, and retracts when done. It
has no "planning" fan-out, because Core emits no plan signal.

### Shortcuts

⌘K command palette (actions and tasks) · ⌘N new task · ⌘\ sidebar.

### Revision after the first live review (25 September 2026)

- The inset glass sidebar clashed with the traffic lights and read as a
  second window. The sidebar is now flush and full height, Codex-style, in
  solid `--sidebar-bg` with a right hairline. Glass is kept for floating
  layers only: the palette, the toast and the full-view toolbar.
- The workspace is near-black (`--l0` #0a0a0b), not a void. The blue ambient
  pools are gone, and only Home keeps a neutral light behind Pegoles.
- Messages read as a conversation. Your request is a bubble on the right.
  Pegoles answers from the left with a 22 px mark, one line and plain
  15 px text. Work groups hang from the same axis. Times appear on hover,
  and the task title is always in the title bar.

## 9. Redesign: work sessions, a first-class computer, graphite and signal (25 September 2026, second pass)

The §8 evolution was clean but still read as a dark chat app. A
four-part audit (live screenshots, 25 shell-lab captures, reference
synthesis against Codex / Apple Liquid Glass / agent-computer products,
motion inventory, technical feasibility) found the problems below. This
pass keeps the state layer, IPC and every honest-state rule; it changes
the shell, the screens, the tokens and the motion.

### Diagnosis

- **Chat, not work.** The request was a bubble and the task page ended in
  a composer that looked like "reply here" but created a *new* task and
  navigated away (Core has no follow-up command).
- **The blockers were footnotes.** "No model connected" was a 12 px line
  under a composer that looked ready; the approval card was barely heavier
  than the composer; the screen that can't be shown was a small caption.
- **The same fact, three or four times.** Steps appeared in the work fold,
  the Thought Ribbon, the hover list and the panel's recent actions; state
  words drifted (*Running / Needs your approval / Needs approval /
  Waiting for your permission*); the computer control appeared in four
  places, some of them without its state.
- **The computer was a setting.** A title-bar pill opened an inspector; off,
  it was a small drawing; Pause and Stop were grey footer links; full view
  was offered over an empty frame; "Take control" disappeared silently.
- **No material, no identity.** A flat #111113 sidebar, one grey, blue
  used almost nowhere, 13–15 px type everywhere, a centred mascot over
  "What now?".
- **Motion bugs.** `[data-reduced-motion]` matched `"false"` so page
  staggers never played; the inspector's reveal ended 20 ms before its
  column, so the native view could appear early and jump; every navigation
  animated `filter: blur`; the Home intro replayed on every return;
  full view kept the screen hidden 580 ms.

### Direction: graphite and signal

Matte graphite for the work; one layer of glass for navigation and
controls; **light that follows agency** — blue wherever Pegoles is acting,
amber wherever you are needed or in control, green when done, red when it
failed. The computer is drawn as a real object whose light is its state.
Tokens and rules: DESIGN_SYSTEM.md §0.

The inset glass sidebar returns, answering §8's objection rather than
ignoring it: the traffic lights now sit *inside* the pane's top-left corner
(pane inset 8, lights at 20, 19), the pane is a dark tint over a lit
atmosphere (light from above, the machine's glow at its foot) instead of a
lighter card, and its radius is concentric with the window. This needs a
look in the live window before it is final.

### Information architecture

```
┌ sidebar (glass pane, inset 8) ┐┌ title bar: place · title/state on scroll ·· [computer capsule] ┐┌ computer bay ┐
│ Pegoles · search              ││                                                                 ││ (Side/Focus) │
│ [+ New task ⌘N]               ││  work column: Home desk | Task session | Activity | Settings   ││              │
│ NEEDS YOU / WORKING /         ││                                                                 ││              │
│ NOT STARTED / DONE            ││                                                                 ││              │
│ ─ machine card (its light) ─  ││                               [ Now dock ] (task only)          ││              │
│ Activity · Settings           ││                                                                 ││              │
└───────────────────────────────┘└─────────────────────────────────────────────────────────────────┘└──────────────┘
```

- **Home — the delegation desk.** Presence, one sentence of where things
  stand ("Pegoles is here, but can't start tasks yet."), *Give Pegoles a
  job.*, the composer with its context underneath (its computer and state,
  the safety rules, "No model" when true — facts that open the panel or the
  right Settings section, never pickers for choices Core can't make), one
  strong next action when a task needs you, then **Continue** (or jobs
  that fit an isolated computer, each saying what it touches) beside **Its
  setup** (the machine with its next real step, the model, the rules).
- **Task — a work session.** The brief (the job as given, the page's one
  heading, with its state pill and counts) → the **run** on one spine
  (actions, files, approvals in their own shapes; a ghost spine explains
  how a run reads before anything happens) → a **result** block once it
  settles (what it made) → the **Now dock**, a floating glass bar at the
  foot: Pegoles, what it is doing now, and the ways to step in ("Watch its
  computer", "New task" once settled). The approval's meaning lives in the
  dock ("Nothing has been allowed…"); the run marks where it stopped.
  No composer on a task: it would be a dead or misleading control.
- **Computer — four levels, one mounted panel.** closed (the machine's
  light in the sidebar, the title bar capsule on a task, the dock's
  Watch) → **Side** (the bay beside the work) → **Focus** (its screen
  first, the run narrow beside it) → **Full** (the whole window). The title
  bar capsule becomes a Side · Focus · Full switch with a gliding lens.
  Focus and Full are only reachable when there is a screen; Esc steps back
  one level. Every state is drawn in the same frame: dark with a power
  glyph when off, brightening through the real boot stages, then the live
  screen or the honest placeholder. Pause / Resume / Stop sit under the
  screen; when taking over isn't possible it says why.
- **Activity** and **Settings** keep their structure with the new type
  and material; Settings can be opened at Security or Pegoles from Home
  and the composer; the ⌘K/⌘N shortcut copy is fixed.

### Motion system

| Moment | What moves | Timing | Reduced motion |
|---|---|---|---|
| Launch | sidebar, title bar, work fade in | 240 ms `out` | fade |
| Home arrival (once per session) | status, headline, lead, composer, lower rise 6–12 px | 700 ms `out`, 60 ms stagger | none |
| Hand-off | words fly field → brief title; Pegoles flies hero → dock | scene 650 ms `move` (shared layout) | crossfade |
| Navigate | view opacity + 6 px | 240 ms in / 120 ms out, no blur | opacity |
| Computer opens | column + clip reveal from the right, content at +80 ms | 320 ms `panel`; native view gated on `pc-reveal` | fade 160 ms |
| Side ↔ Focus | column resize; native view gated on `pc-resize` (340 ms) | 320 ms `panel` | snap |
| Full | backdrop fade, frame un-clips, bar settles | 200 / 320 / 240 ms (+80) | fade |
| Computer closes | retracts to the right while the work widens | 200 ms `exit`, then unmount | instant |
| Boot | frame glow follows real stages, scan line | 700 ms light, 2.2 s loop | static |
| State change | pill words cross over; tone light eases; dock edge recolours | 240 ms / 420 ms | colour only |
| Approval | amber sweep once across the dock and the Home card | 1.4 s once | none |
| Done | check draws, result block rises 6 px | 420 ms | none |
| Selection | sidebar lens, palette capsule, view-switch lens glide | spring `snappy` 0.28 s | instant |
| Press | scale 0.96–0.98 | 80 ms | none |

Rules kept: transform / opacity / clip-path only (columns are the one
layout transition, and the native slot re-measures on `transitionend`);
no `layout` animation on anything a column moves (the state pill lost its
width morph for that reason); no motion on an ancestor of the slot.

### Validation (shell lab, 1440×900 and 1100×760, system Chrome)

Captured every scenario plus Focus, Full, booting, Reduce Transparency,
Increase Contrast and reduced motion; film strips of open, Side → Focus
and close. Desktop tests 140/140 (plus @pegoles/ui 175/175), typecheck and lint clean, production
build: entry 114.4 kB gz (budget 150), CSS 20.1 kB gz. Not verifiable
here: the live Tauri window (no Screen Recording permission), traffic
light alignment against the inset pane, WebKit's rendering of the glass,
and a real native VM view in the frame (the embed is still a stub).

# Pegoles design system

## 0. Desktop shell (September 26 2026 rebuild) — canonical

The production desktop shell has its own token set in
`apps/desktop/src/styles/tokens.css`, mirrored for Motion in
`apps/desktop/src/lib/motion.ts`. Everything from §1 on describes the
Flux Glass library in `@pegoles/ui`, which the shell uses only for runtime
services (`FluxGlassRoot`: effects tier, ambient gate, reduced motion; the
agent cursor overlay). Its glass components remain for the design lab.

**Product truth.** Pegoles is an agent working on its own isolated
computer; the person supervises a worker, they don't chat with one. Every
surface serves the loop *give a job → it works (maybe on its computer) →
you watch, step in or approve → result*.

**Information architecture** (Codex-grade structure, Pegoles identity):

| Region | Contents |
|---|---|
| Sidebar (left, 260 px, navigation material) | traffic-light row + hide toggle · Pegoles (living mark = global state) + Search ⌘K · New task ⌘N · Activity · tasks grouped *Needs you → Working → Today → Yesterday → Previous 7 days → Earlier* (`state/taskSections.ts`) · foot: Computer (device row with live state; toggles the panel) · Settings |
| Work (centre, matte) | 52 px toolbar (drag region; task title once the objective scrolls away; Computer toggle ⌘J with a state light) · the view · the **work bar** at the bottom: the composer on Home, the job's status bar on a task — one place, never both |
| Computer (right, contextual) | one mounted panel for Side / Focus / Full; closed it lives in the sidebar row, the toolbar toggle, the composer's context tab and (while Pegoles uses it) a corner **peek** |

**Screens.** Home is a work surface: the mark, "What should Pegoles do?",
one line of product truth, starters only on first run, the composer
anchored at the bottom, a *Needs you* banner above it when relevant. A
task is a work session: objective (h1) · state and facts · the narrative
(finished runs fold into one sentence such as "Looked at the screen,
clicked and typed text · 6 actions · 2.1s", consequential and failed steps
stay visible, the run in progress stays open) · files it made · where it
stopped to ask · how it ended. What Pegoles is doing *now* lives only in
the status bar (mark, one held line with a light passing through it while
live, elapsed time, and only real interventions: Stop = `cancel_agent_input`
while input is in flight, Watch, New task). No reply box on a task: Core
has no follow-up command. No Approve button: Core has no approval command.

**Computer levels** (`computer/layout.ts`, `useComputerLevel.ts`).
Widths are pixels computed from the window width and applied as the shell
grid's columns (`--col-sidebar`, `--col-computer`), so Side ↔ Focus ↔ Full
is one interpolated spatial motion of the same object (`--dur-spatial`
420 ms, `--ease-spatial`, a sampled critically damped spring). Opening and
closing keep the panel's contents at their open width, so it slides with
its edge instead of reflowing. The native framebuffer slot stays mounted
across levels and is hidden (`obscured`) while the workspace moves. When
there is no live native view, the screen shows a real snapshot from
`capture_screen` (refreshed at most every 1.5 s after Pegoles acts, every
8 s otherwise, only while visible) with its age; no picture, no mock.

**Colour.** Surfaces `--window` #111214 (underlay) · `--surface` #17181A
(work) · `--surface-2` #1D1E21 (inspector, grouped forms) · `--raised`
#25262A (composer, cards) · `--raised-2` #2E2F34 (menus). Text #E6E7E9 at
100 / 66 / 48 % (4.6:1 minimum for meta). Hairlines 8 % (0.5 px where the
platform draws them). Fills: hover 6 %, pressed 10 %, selected 7.5 %.

**Signal.** Colour is light with a meaning: Pegoles blue `#3591FF`
(text-safe `#86BFFF`) = Pegoles is acting, and focus; amber = you
(approvals, your hands on its computer); green done; red failed; grey
idle / not started. One vocabulary (`lib/taskState.ts`).

**Material.** Only the navigation layer is translucent: the sidebar
(tint over the window underlay, 28 px blur), the status bar, palette and
toast (floating material), the composer (a 92 % raised fill with a light
blur). Content — the narrative, files, the computer's screen, settings
groups — is matte. Never material on material. Reduce Transparency and
the Minimal tier make every material solid.

**Type.** System face. Home question 26/32 medium; objective and page
titles 20/26 semibold; UI 13/18; sidebar rows 13.5; reading 14/21; meta
12/16; mono 11.5/16 for paths and durations. Sentence case everywhere; no
eyebrow capitals.

**Radius** (concentric). Composer 22 → its controls 14; screen frame 12 →
screen 8; rows 8 inside an 8 px gutter; cards 12; menus 14.

**Motion** (`lib/motion.ts`). *Micro* 90–180 ms (hover, press scale
0.97 / icons 0.92, selection lens spring 0.2 s with a trace of bounce) ·
*surface* 220 ms (things appearing or changing in place: narrative items
rise 8 px, status line swaps, approval pulse once, outcome tile pops) ·
*spatial* 420 ms (columns, Computer levels, the drawer). Home → task is one
scene: Pegoles' mark travels from Home into the status bar (shared
`layoutId`), the composer yields its place to the status bar, the
objective rises in. Nothing animates `layout` inside a column that is
itself moving. Reduced motion removes movement and keeps fades.

**Honest-state constraints.** No model runner (tasks stay *Not started*,
said once, with the real ways forward), no approval or follow-up command,
no native live view (snapshots instead; taking over says why it isn't
offered).

## 1. Philosophy

Pegoles is an operating environment where an AI gets its own computer.
Flux Glass is its visual language:

- **Mostly void.** The canvas is near-black (`Void #02040A`). Structure is
  one step lighter (`Surface #070B14`). Depth comes from lightness and a
  specular top edge, not from heavy borders or shadows.
- **Blue is presence, never decoration.** Blue appears where Pegoles is:
  listening (focused command field), working (electric edge, TaskController),
  focused (the cyan focus ring is the mark's eye-light). A ready computer
  gets a *faint* blue edge; there is no permanent giant glow anywhere.
- **Status is never blue and never color-only.** Success/warning/danger/
  waiting use green/amber/coral/orchid, and every status has a distinct
  shape and a text label.
- **Glass floats; structure stays solid.** Translucent materials are for
  layers that float over moving content (toolbars, the CommandBar, the
  TaskController, popovers). Rails and panels are solid surfaces.
- **Motion explains or does not happen.** Keyboard-summoned surfaces appear
  instantly; state changes ease out; spatial moves use critically damped
  springs; ambient life sleeps whenever nobody can see it.
- **Honest UI.** No fake percentages, no fake cursors, no simulated state in
  production (section 16).

Reference posture: macOS materials restraint + Linear's keyboard-first calm,
not a SaaS dashboard, chat clone, crypto dashboard or neon gaming UI.

## 2. Palette (exact)

Raw colors live in `packages/ui/src/tokens/palette.ts` (brand) and
`tokens/signal.ts` (status hues). They are referenced **only** by semantic
tokens; component code/CSS never contains a raw color (enforced by
`tokens.test.ts`).

| Token | Hex | Role |
|---|---|---|
| `void` | `#02040A` | Canvas; also the guest compositor background |
| `surface` | `#070B14` | Structure |
| `blueDeep` | `#002A8E` | Depth (electric glass tint) |
| `blueShadow` | `#0139C2` | Shade of the primary fill |
| `pegolesBlue` | `#015FF8` | Presence; primary fills (never text: 3.9:1) |
| `electric` | `#169CFD` | Energy: active edges, accent text (7.0:1) |
| `cyan` | `#4CCEFC` | Glow core, focus ring |
| `ice` | `#97EBFD` | Eye light, highlights |
| `text` | `#F4F7FC` | Primary text (19.1:1 on Void) |
| `muted` | `#8B96A8` | Secondary text (6.9:1 on Void) |

| Signal | Hex | Hue | Status |
|---|---|---|---|
| `green` | `#4FD69C` | 154° | success |
| `amber` | `#F5B545` | 38° | warning |
| `red` | `#FF6F66` | 4° | danger |
| `orchid` | `#DB8CEB` | 290° | waiting on the user |

"Blue" for the no-blue-status test = saturated hue in 165°–270°
(`isBlue`, tested against the brand blues as a sanity check).

## 3. Semantic tokens

Defined in `tokens/semantic.ts` as references (`base` + optional `mix` +
`alpha`). Emitted as CSS custom properties `--pg-<group>-<name>` plus a
channel twin `--pg-<group>-<name>-rgb` ("r g b") for `rgb(var(--x-rgb) / a)`.

| Token | Resolves to | CSS variable |
|---|---|---|
| background.primary / secondary | void / surface | `--pg-bg-primary`, `--pg-bg-secondary` |
| surface.default | surface | `--pg-surface-default` |
| surface.elevated | surface → text 4.5% (`#12161E`) | `--pg-surface-elevated` |
| surface.raised | surface → text 8% | `--pg-surface-raised` |
| surface.sunken | void | `--pg-surface-sunken` |
| text.primary / secondary | text / muted | `--pg-text-primary`, `--pg-text-secondary` |
| text.vibrantSecondary | muted → text 60% (secondary on clear glass) | `--pg-text-vibrant-secondary` |
| text.disabled | muted @ 55% | `--pg-text-disabled` |
| text.onAccent / accent / inverse | text / electric / void | `--pg-text-on-accent`, `--pg-text-accent`, `--pg-text-inverse` |
| accent.primary / active / soft | pegolesBlue / electric / pegolesBlue @16% | `--pg-accent-primary`, `--pg-accent-active`, `--pg-accent-soft` |
| accent.glow / deep / shadow / highlight | cyan / blueDeep / blueShadow / ice | `--pg-accent-glow` … |
| status.success / warning / danger / waiting | green / amber / red / orchid | `--pg-status-*` |
| statusSoft.* | same @14% (pill backgrounds) | `--pg-status-soft-*` |
| glass.regular / clear / electric | surface / surface / surface → blueDeep 35% | `--pg-glass-tint-*` |
| border.subtle / default / strong | text @6% / 10% / 18% | `--pg-border-*` |
| stroke.specular / electric / user | text @16% / electric @50% / text @72% | `--pg-stroke-*` |
| focus.ring / halo | cyan / electric @30% | `--pg-focus-ring`, `--pg-focus-halo` |
| shadow.color | void | `--pg-shadow-color` |

Contrast guarantees (tests): text primary ≥ 7:1 and secondary/accent text ≥
4.5:1 on every opaque surface; white on the primary fill 4.9:1; every status
color ≥ 4.5:1 on Void, Surface and Elevated; focus ring ≥ 3:1.

## 4. Typography

**Pairing: the platform system face + the platform monospace. No bundled
web fonts.**

- UI: `-apple-system, BlinkMacSystemFont, "SF Pro Text", …, "Segoe UI
  Variable Text", "Segoe UI", system-ui`. Pegoles sits beside native traffic
  lights and native Liquid Glass controls (the control pill over the VM
  framebuffer), so the web chrome must speak the same typographic voice. The
  system face ships optical sizing (Text/Display cuts switch automatically)
  and tracking tables, costs zero bytes and zero layout shift.
- Technical data: `ui-monospace, "SF Mono", …, "Cascadia Mono", Menlo,
  Consolas`. This is where identity lives: instrument-style caps labels,
  timings, dimensions and IDs with tabular figures.

A bundled display face was considered and rejected: two families max, and
a third-party display face next to macOS system chrome reads as a web page,
not an environment.

Scale (`tokens/typography.ts`, classes `.pg-type-<role>`), three weights
(400/500/600), tracking negative as size grows:

| Role | Size/Line | Weight | Tracking | Family |
|---|---|---|---|---|
| display | 34/40 | 600 | −0.024em | sans |
| title1 | 26/32 | 600 | −0.020em | sans |
| title2 | 19/25 | 600 | −0.012em | sans |
| headline | 15/21 | 600 | −0.008em | sans |
| body | 14/21 | 400 | −0.006em | sans |
| callout | 13/19 | 400 | −0.003em | sans |
| footnote | 12/17 | 400 | 0 | sans |
| caption | 11/15 | 500 | +0.01em | sans |
| label | 10.5/14 | 500 | +0.08em, uppercase | mono |
| data | 12/16 | 400 | 0, tabular | mono |

## 5. Spacing, radius, elevation

**Spacing** (`tokens/spacing.ts`) is a 4px grid used as rhythm. Prefer the
intent tokens: `inline 6` (icon↔label), `stackTight 4`, `stack 8`,
`controlX 14`, `surfaceInset 16`, `panelInset 20`, `group 24`,
`section 72`, `page 32`. Tight inside a group, generous between groups.

**Radius** (`tokens/radius.ts`): `xs 4, sm 8, md 12, lg 16, xl 20, xxl 24,
pill`. Roles: controls are capsules; fields 12; surfaces 20; CommandBar and
TaskController share 24 (so the morph only changes size); viewport frame 24;
**framebuffer slot 12** (concentric: 24 − 12 inset). The native
`VZVirtualMachineView` host view must clip to exactly
`radiusRole.viewportSlot` (12 pt).

**Elevation** (`tokens/elevation.ts`): shadows in the void color, used only
to separate floating layers: 1 (controls), 2 (floating surfaces), 3 (top
layer: CommandBar, TaskController, popovers). `--pg-shadow-1..3`.

## 6. Materials and the GlassBackend

A material = semantic tint × per-tier recipe (backdrop blur + scrim
opacity) × edge treatment (`tokens/materials.ts`, `components/glass.css`).

| Material | Use | Edge |
|---|---|---|
| regular | Floating chrome over moving content (toolbars, CommandBar, menus) | 10% hairline + specular top rim |
| clear | Small high-signal chips over rich backdrops; primary-weight content only (secondary text switches to the vibrant variant automatically) | 18% hairline |
| electric | Pegoles present and working (TaskController) | electric rim, cyan specular, soft blue glow |

Readability is proven, not hoped for: the scrim of every material × tier
keeps primary text ≥ 4.5:1 over **Ice** and secondary text ≥ 4.5:1 over
**Cyan** — the brightest lights Pegoles ever places behind glass (the VM
framebuffer can never be under web glass; the native view sits above the
webview). `prefers-reduced-transparency` swaps every material to a 97%
solid surface without blur; `prefers-contrast: more` strengthens borders.

**GlassBackend architecture**

- React code uses `GlassSurface` (or the glass hook inside CommandBar /
  TaskController) only. Backend = **css** for web surfaces on every OS.
- **native-macos** (Liquid Glass `NSGlassEffectView`, macOS 26 SDK, public
  API; fallback `NSVisualEffectView`) is used only for native overlays that
  must sit over the VM framebuffer — e.g. the "You're controlling Pegoles
  Computer · Return to Pegoles" pill (Stream A).
- **Windows Mica/Acrylic**: documented future backend; no Windows native
  work in Phase 4.
- A transparent WKWebView (to put `NSGlassEffectView` *behind* web content)
  requires `drawsBackground` KVC / Tauri `macOSPrivateApi` — private API,
  therefore out.

## 7. Effects tiers

`EffectsTier = "full" | "reduced" | "minimal"` (`tokens/effects.ts`).

| Parameter | Full | Reduced | Minimal |
|---|---|---|---|
| Backdrop blur regular · clear · electric | 28 · 18 · 28 px | 14 · 10 · 14 px | none |
| Scrim regular · clear · electric | 82 · 60 · 90 % | 86 · 66 · 92 % | 94 · 90 · 96 % |
| Saturation | 160% | 130% | 100% |
| Glow intensity | 1.0 | 0.7 | 0.4 |
| Specular rim | 1.0 | 0.85 | 0.7 (static) |
| Ambient life | 6 s loop, amplitude 1 | 8 s loop, amplitude 0.5 | off |
| AgentCursor trail | 10 samples | 3 samples | none |
| Transitions | rich (springs, morphs) | standard (springs, morphs) | simple (crossfades) |
| Blur budget (visible backdrop surfaces) | 6 | 3 | 0 |

Minimal is still Pegoles: near-opaque translucent surfaces, crisp strokes,
the specular rim, restrained static glow — just no blur and no continuous
motion. The low-end reference machine (8 GB / 4 cores → Reduced) keeps glass
and presence with ≤ 16 px blur, a 3-sample trail, a single slow ambient
breath and no background animation (tested).

**Selection** — `selectEffectsTier({ recommended, userOverride,
prefersReducedMotion, lowPower })`, pure and tested:

1. A user override (Settings) always wins, including on Low Power.
2. Otherwise the ResourceGovernor recommendation (Rust
   `recommend_effects` in `pegoles-computer` governor, Stream B:
   ≤ 4 GB or ≤ 2 cores or critical pressure → Minimal; 8 GB / 4 cores →
   Reduced; battery nudges down). Before it answers: `DEFAULT_EFFECTS_TIER`
   = Reduced (premium, cheap, never a downgrade flash).
3. Low Power caps Full → Reduced.
4. Reduced motion is **orthogonal**: it is passed through untouched, never
   lowers the tier and is never written back — Pegoles never changes
   accessibility settings.

The app feeds the resolved tier to `FluxGlassRoot`; the Rust enum's wire
form is expected to be `"full" | "reduced" | "minimal"` (`isEffectsTier`
validates it).

## 8. Motion

Durations (`tokens/motion.ts`): `instant 100` (press feedback),
`interaction 180` (hover, focus light, crossfades), `standard 260`,
`fluid 420` (energy edge, surface state), `scene 650` (rare scene changes),
ambient loops 4–8 s.

Easing: `out cubic-bezier(0.23,1,0.32,1)` for entering/state changes,
`inOut cubic-bezier(0.77,0,0.175,1)` for on-screen movement, `drawer
cubic-bezier(0.32,0.72,0,1)`, `ambient cubic-bezier(0.37,0,0.63,1)`, `linear`
for sweeps. Never `ease-in` on UI.

Springs (motion `visualDuration` + `bounce` ≈ Apple response + damping):
`snappy 0.2/0`, `standard 0.4/0`, `morph 0.45/0`, `momentum 0.3/0.2` (only
after a flick), `scene 0.6/0`, `pointer 0.32/0.04`.

Rules: animate only `transform`, `opacity`, `clip-path` (and `filter`
sparingly); no animation on 100+/day keyboard actions (the CommandBar has no
open/close animation); state-change light (focus edge, energy edge) is an
opacity fade on a pre-rendered layer, never a `box-shadow` transition;
hover is gated to `(hover: hover) and (pointer: fine)`.

## 9. Reduced motion

Reduced motion removes movement, never information:

- Press feedback: brightness/background change instead of `scale(0.97)`.
- CommandBar → TaskController: 180 ms crossfade instead of the shared-layout
  morph (`resolveTransitionStyle` → `crossfade`).
- Indeterminate progress: a segmented line breathing in place instead of a
  moving light (segmented so it never reads as a full bar).
- Ambient loops (`.pg-ambient`): off; presence is static brightness.
- `FluxGlassRoot` sets motion's `MotionConfig reducedMotion` accordingly.

Source: the OS `prefers-reduced-motion` (live). The `reducedMotion` prop on
`FluxGlassRoot` exists only for previews (design lab).

## 10. Idle GPU rule and the ambient gate

No continuous full-screen canvas, animated gradients, mouse-follow shaders
or 60 fps decorative loops — anywhere. Small ambient loops are allowed only
behind the gate.

`AmbientController` (`runtime/ambient.ts`, run by `FluxGlassRoot`) writes on
the root element:

- `data-effects-tier="full|reduced|minimal"`
- `data-reduced-motion="true|false"`
- `data-ambient="running|paused"` (+ `data-ambient-reason` for diagnostics)
- `data-visibility="visible|hidden"`

Ambient runs only when the tier allows it, the document is visible
(`visibilitychange` covers WKWebView minimize/hide), the host has not
signalled hidden (`windowHidden` prop, e.g. Tauri minimize), the window is
focused, and the app is not idle (no input for `idleAfterMs`, default 60 s,
AND no active work — `busy` prop). Input handling is a timestamp write; the
idle check is one lazily re-armed timeout; attributes are written only when
they change. No React state, no rerender cascade, no JS animation loop.

CSS contract (`styles/base.css`):

- `.pg-ambient` — decorative life. Paused by default; runs under
  `[data-ambient="running"]`; paused when `[data-offscreen="true"]`; removed
  on Minimal and under reduced motion.
- `.pg-work-anim` — communicates real work (indeterminate progress,
  spinners). Runs while visible even if unfocused (a glance must never show
  a frozen "working"), paused when hidden/offscreen, removed on Minimal.

Per-element offscreen pausing: `useOffscreenPause()` / `observeOffscreen()`
share **one** IntersectionObserver that writes `data-offscreen` directly on
the element.

## 11. Glass performance audit

- **Nested blur suppression**: a `GlassSurface` inside another renders as a
  flat tinted layer without `backdrop-filter` (context-based). Buttons never
  blur.
- **Blur budget**: at most `maxBackdropSurfaces` backdrop-filter surfaces
  visible at once (6 / 3 / 0).
- **Dev audit**: in dev builds every blurring surface registers
  (`registerBlurSurface`, `useBlurAudit`) with its label, material, radius
  and onscreen state (via the shared observer). The design lab's status
  strip shows `visible / budget · mounted` and flags overruns with a list —
  no console noise. Production builds skip registration (`IS_DEV`).
- Blur ≤ 28 px (Safari cost grows with radius); glass never animates its
  blur.

## 12. Accessibility rules

- Semantic elements first: `form`, `button`, `ol/li`, `time`, `section` +
  heading, real `role="progressbar"` (no `aria-valuenow` when indeterminate).
- Visible designed focus: 2 px cyan outline, 2 px offset, on every control;
  never removed without replacement.
- Color is never the only cue (StatusIndicator shapes + labels, boot stage
  glyphs + visually hidden status words, ×N repeat counts read as "N times").
- Live regions: ComputerViewport announces state changes politely; errors
  use `role="alert"`; TaskController status is `role="status"`.
- Text on glass ≥ 4.5:1 by construction (section 6); text on opaque
  surfaces tested.
- `prefers-reduced-motion`, `prefers-reduced-transparency`,
  `prefers-contrast` all respected.
- Buttons use the default arrow cursor (macOS convention for an app).

## 13. Components

All presentational; the app (Stream E) owns data and effects.

| Component | Purpose | Rules |
|---|---|---|
| `FluxGlassRoot` | Mount once: token sheet, ambient gate, tier context, lazy motion features | Props: `tier`, `reducedMotion?` (previews only), `busy?`, `windowHidden?`, `idleAfterMs?`, `target?` |
| `EffectsScope` | Subtree with another tier (lab comparisons) | Not for production UI |
| `GlassSurface` | CSS glass backend | `material` regular/clear/electric, `elevation` 0–3, `radius` md/lg/xl/xxl/pill, `as`, `auditLabel`. Floating layers only |
| `GlassButton` | Capsule button | `variant` primary/secondary/quiet, `tone` accent/neutral, `size` sm/md/lg, `icon`, `trailing`, `iconOnly` (children = accessible name), `isLoading`. One primary per view |
| `StatusIndicator` | Shape + color + word | `tone` neutral/active/success/warning/danger/waiting/paused/user, `label` (required), `detail`, `pill`, `live`, `pulse` (active only, ambient), `hideLabel` |
| `CommandBar` | Signature command field | `onSubmit(text)`, controlled `value`/`onValueChange` or uncontrolled, `placeholder` "Ask Pegoles…", `isBusy`, `leading` (PegolesMark), `shortcutHint`, `layoutId`. Enter sends, Escape clears then blurs, IME-safe |
| `TaskController` | What the CommandBar becomes | `title`, `status {tone,label}`, `detail`, `elapsed`, `leading`, `actions`, `working`, `progress` (real only) |
| `CommandMorph` | The transition between the two | `mode` command/task, `command`, `task` nodes |
| `ProgressLine` | Hairline progress | `value` fraction or `null` (indeterminate), `label`, `showValue` |
| `ActivityItem` / `ActivityList` | Calm timeline row | `kind`, `title`, `detail`, `at`, `timeLabel`, `tone`, `repeatCount` (coalesce repeats), `emphasis` |
| `ComputerViewport` | Pegoles Computer shell | See below |
| Icons | One family: 16 px grid, 1.5 px stroke | Decorative (`aria-hidden`); name the control instead |

**CommandBar → TaskController morph pattern**

```tsx
<CommandMorph
  mode={task ? "task" : "command"}
  command={<CommandBar onSubmit={createTask} leading={<PegolesMark size={26} state="listening" decorative />} />}
  task={<TaskController title={task.title} status={taskStatus} working={task.running} actions={…} />}
/>
```

Both surfaces carry `COMMAND_SURFACE_LAYOUT_ID`; with `morph` the glass
reshapes in place (motion `layoutId`, `spring.morph`, content kept unscaled
with `layout="position"`), the task content fades in after 80 ms. With
`crossfade` (reduced motion or Minimal) the two swap with a 180 ms opacity
crossfade. Switch `mode` only on a real `create_task` result.

**ComputerViewport** props: `state` (the 10 Core `ViewportState` values,
snake_case), `display {width_px, height_px}` (default 1440×900),
`nativeSurface`, `slotRef` (geometry bridge), `title`, `subtitle`,
`headingLevel`, `bootStages` (real stages: `{id, label, status, detail?,
progress?}`), `statusDetail`, `errorMessage`, `errorActions`, `offActions`,
`headerActions`, `onTakeControl`, `onReturnControl`, `controlButtonRef`,
`returnShortcut` (default ⌃⌥⎋), `placeholder`, `variant` full/compact,
`fit` width/contain.

- The slot keeps the display aspect ratio; with `nativeSurface` it is an
  empty hole — nothing in the DOM paints inside it. Energy edges are sibling
  layers with outer box-shadows only (outer shadows never paint inside the
  border box). Background = Void (matches the guest compositor, so seams are
  invisible).
- Energy edge: off/idle neutral hairline; booting faint (20%); ready very
  restrained blue (34%); agent_active full electric + ambient breath;
  user_controlled neutral white; paused dimmed hairline (native side dims
  the framebuffer); error restrained coral.
- Boot: real stages; indeterminate line when no percentage exists; a
  percentage only when `progress` is a real number.
- Control: "Take control" (ready / agent_active, handler provided);
  "You're controlling Pegoles Computer · ⌃⌥⎋ · Return to Pegoles" while
  `user_controlled`. All affordances live in the footer, never over the slot.
- Messages that would overlay a native surface (boot stage, errors) move to
  the footer automatically.

## 14. Tokens → CSS

`fluxGlassStyleSheet()` renders every token as custom properties (`:root`
= base tokens + default tier, then one `[data-effects-tier="…"]` block per
tier, accessibility media blocks, `.pg-type-*` classes). `FluxGlassRoot`
injects it once (`<style id="pegoles-flux-glass-tokens">`, idempotent,
inserted before first paint). Chosen over a generated static CSS file so the
TypeScript tokens stay the single source of truth with no build step and no
drift; switching tier is one attribute write. Component styles are static
CSS files that consume only these variables (tested: every `var(--pg-*)`
resolves, no raw hex).

## 15. Integration notes

- `@pegoles/ui` re-exports Stream D2's `presence`, `cursor` and `brand`
  modules (PegolesMark, presence machine, AgentCursor, mark geometry).
- `useFluxGlass()` gives `{ tier, reducedMotion, params, transitionStyle }`
  (low-frequency context); `useAmbientState()` for diagnostics/settings.
- Native display (Stream A): clip the framebuffer host view to 12 pt corner
  radius; Weston background `#02040A` equals `--pg-surface-sunken`.

## 16. No fake state in production

Production components render only what Core reports: no invented
percentages (ProgressLine is indeterminate without a real measure), no
"wait N seconds = ready", no simulated cursor or task. Simulated fixtures
exist only under `apps/desktop/src/dev/` and are labelled on screen
("Simulated fixture", dashed tag).

## 17. Design lab

`pnpm --filter @pegoles/desktop dev`, then open `http://localhost:1420/#/dev/design`.

- Dev only: `main.tsx` loads it through `import.meta.env.DEV` + dynamic
  `import()`, so production builds drop it. `src/dev/prodBundle.test.ts`
  fails if `apps/desktop/dist` contains the lab marker
  (`__PEGOLES_DESIGN_LAB__`); run it after `pnpm --filter @pegoles/desktop
  build`.
- Environment panel: effects tier (Auto/Full/Reduced/Minimal), governor
  fixture (24 GB → Full, 8 GB → Reduced, 4 GB → Minimal, pending), motion
  (system/reduce/allow), Low Power. The glass status strip shows the
  resolved tier and its source, the ambient gate and the blur budget (click
  it for the surface list).
- Hash parameters for reproducible screenshots:
  `#/dev/design?tier=minimal&motion=reduce&governor=lowEnd&section=computer&state=agent_active`.
- Sections: color, typography, materials, buttons, status, command → task,
  activity, Pegoles Computer (all 10 states, native-hole toggle, boot
  sequence), motion, effects tiers, presence and agent cursor (Stream D2).

## 18. Desktop shell integration

The production shell now mounts `FluxGlassRoot` and uses the existing mark
traced from the supplied PNG (the source asset is byte-identical). The 76 px
rail contains Home, Agents, Computer, Activity and Settings. Home centers a
single floating CommandBar above task rows; opaque content surfaces keep
glass confined to navigation and command/control surfaces. System fonts
use the host's SF Pro / Segoe UI stack, with a 34–49 px Home heading and
compact 11–15 px supporting hierarchy.

- `create_task` / `list_tasks` supply task content. Pending explicitly means
  execution unavailable, and no thinking animation is inferred from it.
- The selected task workspace provides split, computer and conversation
  layouts. Hidden panes are inert and hidden from assistive technology.
  Grid transitions use 420 ms; native surfaces animate through the existing
  public geometry command instead of per-frame IPC. Minimal/system reduced
  motion removes spatial transitions.
- `get_status.viewport_state` is passed directly to ComputerViewport, with
  no frontend inference from VM or guest state. Until Core responds, the
  viewport shows an unavailable message rather than guessing Off or Ready.
- NativeComputer serializes geometry/detach commands, observes slot size,
  debounces layout/scroll events, and detaches on navigation or loss of
  availability. It uses the existing native adapter without any Rust edits.
  Take control / Return to Pegoles use existing commands; ownership is only
  reported after Core responds. No autonomous cursor is mounted.
- `suggested_effects` supplies Auto. Reduced is the fallback until it replies.
  Full/Reduced/Minimal overrides persist locally; system reduced motion is
  independent and always honored.
- Event listeners refresh snapshots; a 2.5 s visible-window poll reconciles
  guest/display state and recovers missed events. Browser previews never
  invoke native commands or invent tasks. Event rows render the latest 100.
- Keyboard access includes a skip link, visible focus, native controls and
  Cmd/Ctrl+K for Home's command input. Page navigation moves focus to the
  heading. Responsive layouts stack below 760 px without adding mobile work.

The dev lab is available at `/dev/design` as well as `#/dev/design`; both
remain guarded by `import.meta.env.DEV` and absent from production bundles.

- Shell v2 (monochrome, Codex layout + Grok atmosphere): a 268 px task
  sidebar (New task / Cmd+K, live search, Active and Recent groups with
  status glyph + relative time, a spinner only on `running`), places
  (Computer, Activity, Settings) and the Core connection at the bottom.
  Below 860 px the sidebar is a drawer. The shell is black/white/grey;
  colour is only the Pegoles mark and status (never colour-only).
- Home: companion mark (128 px) that floats, follows the pointer with its
  eyes, looks at the composer and hops when listening; greeting by time
  of day; the title resolves word by word out of a blur; a composer with a
  light that circles its edge while focused (faster while creating);
  suggestion chips that only fill the composer. Ambient: drifting stars,
  a slow sheen and an eclipse horizon that brightens with presence, plus
  static grain. All loops ride the ambient gate; Minimal/reduced motion
  remove them.
- Task thread: your request as a bubble, Pegoles' turn with the live
  action (shimmer only while really working, `.pg-work-anim`), the steps
  timeline (chronological; only in-flight actions are current), the honest
  pending notice, and the computer beside it (Split / Compact / Focus with
  a gliding pill).
- Views enter by rising out of a soft blur; no exit animations, so
  navigation stays instant. Settings uses a `SegmentedControl` (radio
  group, arrows, gliding thumb) and shows the live system reduced-motion
  value.

### Integration limits

No model execution is provided: real submitted tasks remain pending. Native
framebuffer and Liquid Glass behavior depend on the existing platform adapter;
a browser preview cannot validate those native surfaces. The frontend does not
implement networking, VM internals, a renderer, or autonomous input. Host
recommendations are fetched on mount; automatic live power-pressure updates
are not exposed through the current frontend integration.

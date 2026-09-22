/**
 * Pegoles Flux Glass — motion tokens (Phase 4 source of truth).
 * Durations in milliseconds. Spatial motion prefers springs; these
 * durations bound tweens and spring settle targets.
 */
export const duration = {
  instant: 100,
  interaction: 180,
  standard: 260,
  fluid: 420,
  scene: 650,
  /** Ambient loops (breathing etc.): 4–8 s window. */
  ambientMin: 4000,
  ambientMax: 8000,
} as const;

export type DurationToken = keyof typeof duration;

/**
 * Easing curves. No `ease-in` for UI: it delays the frame the user is
 * watching. Curves from easing.dev / Ionic, not hand-rolled.
 */
export const easing = {
  /** Enter/exit and state changes: strong ease-out. */
  out: "cubic-bezier(0.23, 1, 0.32, 1)",
  /** On-screen movement from A to B. */
  inOut: "cubic-bezier(0.77, 0, 0.175, 1)",
  /** Sheets and drawers (iOS-like). */
  drawer: "cubic-bezier(0.32, 0.72, 0, 1)",
  /** Ambient breathing (easeInOutSine): no perceptible start/stop. */
  ambient: "cubic-bezier(0.37, 0, 0.63, 1)",
  /** Indeterminate progress sweeps. */
  linear: "linear",
} as const;

export type EasingToken = keyof typeof easing;

/** Same curves as cubic-bezier tuples for `motion` transitions. */
export const easingBezier = {
  out: [0.23, 1, 0.32, 1],
  inOut: [0.77, 0, 0.175, 1],
  drawer: [0.32, 0.72, 0, 1],
  ambient: [0.37, 0, 0.63, 1],
} as const satisfies Record<string, readonly [number, number, number, number]>;

export interface SpringPreset {
  readonly type: "spring";
  /** Seconds — Apple's "response": time to visually arrive. */
  readonly visualDuration: number;
  /** 0 = critically damped (Apple damping 1.0); 0.2 ≈ damping 0.8. */
  readonly bounce: number;
}

/**
 * Spring presets (motion's visualDuration + bounce ≈ Apple's response +
 * damping ratio). Critically damped by default; bounce only where a
 * gesture carried momentum.
 */
export const spring = {
  /** Small, direct feedback (toggles, pucks). */
  snappy: { type: "spring", visualDuration: 0.2, bounce: 0 },
  /** Move / reposition (Apple PiP: damping 1.0, response 0.4). */
  standard: { type: "spring", visualDuration: 0.4, bounce: 0 },
  /** CommandBar → TaskController shared-layout morph. */
  morph: { type: "spring", visualDuration: 0.45, bounce: 0 },
  /** Released after a flick/drag (Apple drawer: damping 0.8, response 0.3). */
  momentum: { type: "spring", visualDuration: 0.3, bounce: 0.2 },
  /** Scene-level layout changes (split ↔ focus). */
  scene: { type: "spring", visualDuration: 0.6, bounce: 0 },
  /** Agent cursor travel: arrives decisively, no wobble. */
  pointer: { type: "spring", visualDuration: 0.32, bounce: 0.04 },
} as const satisfies Record<string, SpringPreset>;

export type SpringToken = keyof typeof spring;

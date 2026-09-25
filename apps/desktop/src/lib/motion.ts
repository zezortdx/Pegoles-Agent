/**
 * Motion tokens for the Motion library and imperative animation. Mirrors
 * the --dur-* and --ease-* custom properties in styles/tokens.css.
 * Durations are seconds (Motion's unit).
 *
 * Three tiers, each with a job:
 *   micro   (120–180 ms) — a control answers you: hover, press, selection.
 *   surface (180–260 ms) — something appears or changes state in place.
 *   spatial (280–420 ms) — the workspace recomposes: panels, levels, scenes.
 * Springs are critically damped (bounce 0) except where something is
 * placed by the person (selection), which may settle with a trace of give.
 */
export const duration = {
  microFast: 0.09,
  micro: 0.14,
  microSlow: 0.18,
  surface: 0.22,
  layout: 0.24,
  layoutSlow: 0.3,
  agent: 0.24,
  agentSlow: 0.26,
  spatial: 0.36,
  scene: 0.36,
  sceneSlow: 0.42,
} as const;

type Bezier = readonly [number, number, number, number];

export const ease = {
  /** Enter and state change. */
  out: [0.22, 1, 0.36, 1] as Bezier,
  /** A → B travel, retract and reform. */
  move: [0.65, 0, 0.35, 1] as Bezier,
  /** Panels and columns: fast start, long soft landing. */
  spatial: [0.32, 0.72, 0, 1] as Bezier,
  /** Light, breathing, slow fades. */
  light: [0.37, 0, 0.63, 1] as Bezier,
  /** Exits. Keep them short. */
  exit: [0.4, 0, 1, 1] as Bezier,
} as const;

/** Milliseconds, for timers and native geometry animation. */
export const ms = (seconds: number): number => Math.round(seconds * 1000);

/** Springs start from wherever the element is, so an interrupted change never jumps. */
export const spring = {
  /** Selection lenses and thumbs. */
  micro: { type: "spring", visualDuration: 0.2, bounce: 0.12 },
  /** Pills, capsules, small layout. */
  snappy: { type: "spring", visualDuration: 0.26, bounce: 0 },
  /** Surfaces changing size in place (composer ↔ status bar). */
  surface: { type: "spring", visualDuration: 0.3, bounce: 0 },
  /** Panels, columns, cards changing size. */
  smooth: { type: "spring", visualDuration: 0.4, bounce: 0 },
  /** The Home → task hand-off and other scene moves. */
  scene: { type: "spring", visualDuration: 0.38, bounce: 0 },
} as const;

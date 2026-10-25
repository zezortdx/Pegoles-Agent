import type { PresenceMode } from "./modes";

/**
 * Pose and light targets for one mode, applied as CSS custom properties
 * (CSS transitions ease between them); React only ever changes the mode.
 */
export interface PresenceTargets {
  /** Breathing: fractional scale amplitude (≤ 0.012) and period in seconds. */
  readonly breathAmp: number;
  readonly breathPeriod: number;
  /** Outer shell separating into layered contours, 0..1. */
  readonly shellSep: number;
  /** Thought field contours around the mark, 0..1. */
  readonly field: number;
  /** Luminous filaments moving through the shell, 0..1. */
  readonly filament: number;
  /** Speed of field and filament flow (1 = baseline). */
  readonly flow: number;
  /** Eye scale around each eye's centre (1 = rest). */
  readonly eyeScaleX: number;
  readonly eyeScaleY: number;
  /** Eyelid: fraction of each eye covered from the top, 0..1. */
  readonly lid: number;
  /** Eye emission, 0..1.2. */
  readonly eyeGlow: number;
  /** How much `look`, the pointer and idle glances steer the eyes, 0..1. */
  readonly gaze: number;
  /** Amber attention tint, 0..1. Only for needs-user. */
  readonly warn: number;
  /** Overall dimming, 0..1 (offline). */
  readonly dim: number;
  /** Soft light around the body, 0..1. */
  readonly halo: number;
  /** Idle life: randomized blinks and glances (see life.ts). */
  readonly life: boolean;
}

const REST: PresenceTargets = {
  breathAmp: 0.012,
  breathPeriod: 7,
  shellSep: 0,
  field: 0,
  filament: 0,
  flow: 0,
  eyeScaleX: 1,
  eyeScaleY: 1,
  lid: 0,
  eyeGlow: 1,
  gaze: 1,
  warn: 0,
  dim: 0,
  halo: 0.5,
  life: true,
};

/* Working states read at a glance: contours and filaments are thin and cold,
 * but clearly present even at 48 px. */
const TABLE: Record<PresenceMode, PresenceTargets> = {
  offline: { ...REST, breathAmp: 0, lid: 0.72, eyeGlow: 0.35, gaze: 0, dim: 0.55, halo: 0, life: false },
  idle: REST,
  attentive: { ...REST, breathAmp: 0.008, eyeScaleY: 1.04, eyeGlow: 1.06, halo: 0.62 },
  acknowledging: { ...REST, breathAmp: 0, shellSep: 0.15, field: 0.3, eyeScaleX: 1.06, eyeScaleY: 1.1, eyeGlow: 1.15, gaze: 0.3, halo: 0.75, life: false },
  thinking: { ...REST, breathAmp: 0.006, breathPeriod: 5, shellSep: 0.45, field: 0.72, filament: 0.88, flow: 0.6, lid: 0.08, gaze: 0.25, halo: 0.66, life: false },
  planning: { ...REST, breathAmp: 0.006, breathPeriod: 5, shellSep: 0.6, field: 0.95, filament: 0.92, flow: 0.45, lid: 0.1, gaze: 0.25, halo: 0.66, life: false },
  working: { ...REST, breathAmp: 0.004, breathPeriod: 3.6, shellSep: 0.35, field: 0.8, filament: 1, flow: 1, halo: 0.64, life: false },
  "using-computer": { ...REST, breathAmp: 0.003, breathPeriod: 4, shellSep: 0.25, field: 0.64, filament: 0.96, flow: 0.8, halo: 0.6, life: false },
  waiting: { ...REST, breathAmp: 0, shellSep: 0.05, field: 0.08, lid: 0.1, halo: 0.45 },
  "needs-user": { ...REST, breathAmp: 0.004, breathPeriod: 6, shellSep: 0.1, eyeScaleX: 1.05, eyeScaleY: 1.06, eyeGlow: 1.1, warn: 0.85, halo: 0.55, life: false },
  blocked: { ...REST, breathAmp: 0, lid: 0.46, eyeGlow: 0.75, gaze: 0, halo: 0.3, dim: 0.15, life: false },
  done: { ...REST, breathAmp: 0, lid: 0.12, gaze: 0, halo: 0.55, life: false },
  error: { ...REST, breathAmp: 0, lid: 0.18, eyeScaleX: 0.92, eyeScaleY: 0.9, eyeGlow: 0.62, gaze: 0, halo: 0.2, dim: 0.12, life: false },
};

export interface TargetOptions {
  readonly reducedMotion?: boolean;
}

/**
 * Pure mode → targets table. Reduced motion keeps every state legible
 * through light and eye shape but removes spatial motion: no breathing,
 * no flow, no blinks or glances.
 */
export function presenceTargets(mode: PresenceMode, options: TargetOptions = {}): PresenceTargets {
  const targets = TABLE[mode];
  if (!options.reducedMotion) return targets;
  return { ...targets, breathAmp: 0, flow: 0, gaze: 0, life: false };
}

/**
 * The pose as CSS custom properties. Values are unitless except the periods.
 */
export function poseVariables(t: PresenceTargets): Record<string, string> {
  const flowPeriod = t.flow > 0 ? 4.2 / t.flow : 4.2;
  return {
    "--p-eye-sx": String(t.eyeScaleX),
    "--p-eye-sy": String(t.eyeScaleY),
    "--p-lid": String(t.lid),
    "--p-eye-glow": String(t.eyeGlow),
    "--p-dim": String(1 - t.dim),
    "--p-halo": String(t.halo),
    "--p-shell-sep": String(t.shellSep),
    "--p-field": String(t.field),
    "--p-filament": String(t.filament),
    "--p-warn": String(t.warn),
    "--p-gaze": String(t.gaze),
    "--p-breath-amp": String(t.breathAmp),
    "--p-breath-period": `${t.breathPeriod}s`,
    "--p-flow-period": `${flowPeriod.toFixed(2)}s`,
    "--p-sweep-period": `${(flowPeriod * 1.3).toFixed(2)}s`,
  };
}

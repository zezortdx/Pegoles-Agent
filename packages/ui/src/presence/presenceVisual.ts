/**
 * Presence → visual recipe (pure). Decides which layers of the mark exist,
 * their light levels, and which CSS animations run, for a given presence
 * state × effects tier × reduced-motion preference × rendered size.
 *
 * Rules (Phase 4 plan D6 + presence brief):
 * - Full: rich glow (near + far), specular, traveling light (Thinking),
 *   ambient breathing, directional attention, 1–3 px eye shift.
 * - Reduced: calmer glow (no far halo), slower/smaller breathing, no
 *   traveling light (a static arc that shimmers in opacity instead).
 * - Minimal: static luminous mark — no loops at all; state is carried by
 *   brightness/opacity only.
 * - Reduced motion (any tier): no transform animation, no eye/glow shift,
 *   no loops; state stays readable through light levels + opacity fades.
 * Every animation is opacity or transform only, and every loop is gated
 * (`ambient` → .pg-ambient, `work` → .pg-work-anim) so it sleeps when the
 * window is idle/hidden or the mark is offscreen.
 */
import { effectsTiers, type EffectsTier } from "../tokens/effects.js";
import type { PresenceState } from "./presenceMachine.js";
import { SUCCESS_HOLD_MS } from "./presenceMachine.js";

/** Below this rendered size the mark drops detail it cannot show anyway. */
export const COMPACT_MARK_PX = 32;

export interface PresenceVisualInput {
  readonly state: PresenceState;
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  /** Rendered size in CSS px (default 64). */
  readonly size?: number;
}

export interface PresenceLayers {
  /** Pre-blurred near glow (static bitmap once rasterized; opacity only). */
  readonly glow: boolean;
  /** Wide, faint halo — Full only, and not at compact sizes. */
  readonly farGlow: boolean;
  /** Pre-blurred eye light. */
  readonly eyeGlow: boolean;
  /** Ring-masked energy container (charge / dim / light effects). */
  readonly energy: boolean;
  /** Traveling light around the ring (rotating conic highlight). */
  readonly orbit: boolean;
  /** Static top arc highlight (Thinking where the orbit is not allowed). */
  readonly arc: boolean;
  /** Directional light spot toward the command surface. */
  readonly attention: boolean;
  /** Brief cyan acknowledgement (Success). */
  readonly success: boolean;
  /** Semantic error badge (shape + color + accessible name). */
  readonly badge: boolean;
}

export interface PresenceLevels {
  readonly glow: number;
  readonly charge: number;
  readonly dim: number;
  readonly eyes: number;
  readonly attention: number;
  readonly arc: number;
  readonly specular: number;
}

export type AnimationTarget = "glow" | "orbit" | "arc" | "charge" | "success";
export type AnimationGate = "ambient" | "work" | "once";

export interface PresenceAnimation {
  readonly target: AnimationTarget;
  readonly keyframes: "pgm-breathe" | "pgm-orbit" | "pgm-shimmer" | "pgm-pulse" | "pgm-flash";
  readonly property: "opacity" | "transform";
  readonly durationMs: number;
  readonly iterations: number | "infinite";
  readonly direction: "normal" | "alternate";
  readonly easing: string;
  readonly gate: AnimationGate;
}

export interface PresenceVisual {
  readonly state: PresenceState;
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  readonly compact: boolean;
  readonly layers: PresenceLayers;
  readonly levels: PresenceLevels;
  readonly animations: readonly PresenceAnimation[];
  /** Eyes may drift 1–3 px toward `lookAt`. */
  readonly eyeMotion: boolean;
  /** Glow may lean toward `attention`. */
  readonly glowShift: boolean;
  /** Lower bound of the breathing opacity loop (upper bound is 1). */
  readonly breatheMin: number;
  /** Duration of opacity cross-fades between states. */
  readonly fadeMs: number;
}

interface StateRecipe {
  readonly glow: number;
  readonly charge: number;
  readonly dim: number;
  readonly eyes: number;
  readonly attention: number;
  readonly arc: number;
  readonly breathes: boolean;
}

const RECIPES: Readonly<Record<PresenceState, StateRecipe>> = {
  idle: { glow: 0.72, charge: 0, dim: 0, eyes: 1, attention: 0, arc: 0, breathes: true },
  listening: { glow: 0.84, charge: 0.08, dim: 0, eyes: 1, attention: 0.6, arc: 0, breathes: true },
  thinking: { glow: 0.86, charge: 0.1, dim: 0, eyes: 1, attention: 0, arc: 0.85, breathes: false },
  acting: { glow: 1, charge: 0.32, dim: 0, eyes: 1, attention: 0, arc: 0, breathes: false },
  waitingForUser: { glow: 0.8, charge: 0.06, dim: 0, eyes: 1, attention: 0.9, arc: 0, breathes: false },
  success: { glow: 0.92, charge: 0.1, dim: 0, eyes: 1, attention: 0, arc: 0, breathes: false },
  error: { glow: 0.3, charge: 0, dim: 0.42, eyes: 0.72, attention: 0, arc: 0, breathes: false },
  offline: { glow: 0.06, charge: 0, dim: 0.62, eyes: 0.45, attention: 0, arc: 0, breathes: false },
};

const EASE_AMBIENT = "cubic-bezier(0.37, 0, 0.63, 1)";
const EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";
/** Traveling light: one lap per 2.6 s — unhurried, clearly "working". */
export const ORBIT_PERIOD_MS = 2600;

function round(n: number): number {
  return Math.round(n * 1000) / 1000;
}

export function presenceVisual(input: PresenceVisualInput): PresenceVisual {
  const { state, tier, reducedMotion } = input;
  const size = input.size ?? 64;
  const compact = size < COMPACT_MARK_PX;
  const params = effectsTiers[tier];
  const recipe = RECIPES[state];
  const minimal = tier === "minimal";
  const loopsAllowed = !minimal && !reducedMotion;
  const thinking = state === "thinking";
  const orbit = thinking && tier === "full" && !reducedMotion;

  const layers: PresenceLayers = {
    glow: true,
    farGlow: tier === "full" && !compact,
    eyeGlow: !compact,
    energy: true,
    orbit,
    arc: thinking && !orbit,
    attention: recipe.attention > 0,
    success: state === "success",
    badge: state === "error",
  };

  const animations: PresenceAnimation[] = [];
  if (loopsAllowed && recipe.breathes && params.ambient.enabled && params.ambient.amplitude > 0) {
    animations.push({
      target: "glow",
      keyframes: "pgm-breathe",
      property: "opacity",
      durationMs: params.ambient.periodMs,
      iterations: "infinite",
      direction: "alternate",
      easing: EASE_AMBIENT,
      gate: "ambient",
    });
  }
  if (orbit) {
    animations.push({
      target: "orbit",
      keyframes: "pgm-orbit",
      property: "transform",
      durationMs: ORBIT_PERIOD_MS,
      iterations: "infinite",
      direction: "normal",
      easing: "linear",
      gate: "work",
    });
  }
  if (loopsAllowed && layers.arc) {
    animations.push({
      target: "arc",
      keyframes: "pgm-shimmer",
      property: "opacity",
      durationMs: 2200,
      iterations: "infinite",
      direction: "alternate",
      easing: EASE_AMBIENT,
      gate: "work",
    });
  }
  if (loopsAllowed && state === "acting") {
    animations.push({
      target: "charge",
      keyframes: "pgm-pulse",
      property: "opacity",
      durationMs: tier === "full" ? 2400 : 3200,
      iterations: "infinite",
      direction: "alternate",
      easing: EASE_AMBIENT,
      gate: "work",
    });
  }
  if (state === "success") {
    // A finite brightness acknowledgement: allowed in every tier and under
    // reduced motion (opacity only, no movement).
    animations.push({
      target: "success",
      keyframes: "pgm-flash",
      property: "opacity",
      durationMs: SUCCESS_HOLD_MS,
      iterations: 1,
      direction: "normal",
      easing: EASE_OUT,
      gate: "once",
    });
  }

  return {
    state,
    tier,
    reducedMotion,
    compact,
    layers,
    levels: {
      glow: round(recipe.glow * params.glowIntensity),
      charge: round(recipe.charge),
      dim: round(recipe.dim),
      eyes: round(recipe.eyes),
      attention: round(recipe.attention),
      arc: round(recipe.arc),
      specular: round(params.specular),
    },
    animations,
    eyeMotion: !reducedMotion && !minimal,
    glowShift: !reducedMotion && !minimal && recipe.attention > 0,
    breatheMin: round(1 - 0.35 * params.ambient.amplitude),
    fadeMs: reducedMotion ? 180 : 420,
  };
}

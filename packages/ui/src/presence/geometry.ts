/**
 * Presence geometry helpers (pure): the eyes' subtle 1–3 px drift toward
 * relevant content, and the direction of Listening's light concentration.
 */

/** Hard ceiling for the eye drift, in CSS px, at any size. */
export const MAX_EYE_SHIFT_PX = 3;
/** Drift scales with the mark (≈0.6 px at 20 px, 3 px from 100 px up). */
export const EYE_SHIFT_PER_PX = 0.03;

export interface Vec2 {
  readonly x: number;
  readonly y: number;
}

const ZERO: Vec2 = { x: 0, y: 0 };

function finite(n: number): number {
  return Number.isFinite(n) ? n : 0;
}

/**
 * Eye offset in CSS px for a `lookAt` direction. `lookAt` is a direction
 * (screen space, y down); its length is clamped to 1, so any magnitude is
 * safe. The result never exceeds MAX_EYE_SHIFT_PX in length.
 */
export function eyeOffset(lookAt: Vec2 | null | undefined, sizePx: number): Vec2 {
  if (!lookAt) return ZERO;
  const x = finite(lookAt.x);
  const y = finite(lookAt.y);
  const len = Math.hypot(x, y);
  if (len === 0) return ZERO;
  const k = Math.min(1, len) / len;
  const max = Math.min(MAX_EYE_SHIFT_PX, Math.max(0, finite(sizePx)) * EYE_SHIFT_PER_PX);
  return { x: Math.round(x * k * max * 100) / 100, y: Math.round(y * k * max * 100) / 100 };
}

/**
 * Direction from the mark's centre to a point of interest (both in the
 * same CSS px space). Saturates at `range` px: content farther away gets
 * the full (still tiny) drift, nearer content proportionally less.
 */
export function lookToward(from: Vec2, to: Vec2, range = 240): Vec2 {
  const dx = finite(to.x - from.x);
  const dy = finite(to.y - from.y);
  const dist = Math.hypot(dx, dy);
  if (dist === 0 || range <= 0) return ZERO;
  const strength = Math.min(1, dist / range);
  return { x: (dx / dist) * strength, y: (dy / dist) * strength };
}

/** Default Listening direction: down, toward the command surface. */
export const DEFAULT_ATTENTION_DEG = 90;

/**
 * Normalizes `attention` (degrees, screen space: 0 = right, 90 = down, or
 * a direction vector) to radians.
 */
export function attentionAngle(attention: number | Vec2 | null | undefined): number {
  if (attention === null || attention === undefined) return (DEFAULT_ATTENTION_DEG * Math.PI) / 180;
  if (typeof attention === "number") {
    return (finite(attention) * Math.PI) / 180;
  }
  const x = finite(attention.x);
  const y = finite(attention.y);
  if (x === 0 && y === 0) return (DEFAULT_ATTENTION_DEG * Math.PI) / 180;
  return Math.atan2(y, x);
}

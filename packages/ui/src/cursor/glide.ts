/**
 * Agent cursor travel (pure): one glide from where the cursor IS to where
 * the agent's latest action happened.
 *
 * - Duration grows with the distance on screen, logarithmically (Fitts-like):
 *   a nudge is nearly instant, a cross-screen move takes a little longer,
 *   never more than GLIDE.maxMs. Real input never waits for any of this.
 * - Shape: a cubic Hermite segment with zero end velocity, i.e. ease-in-out
 *   from rest. Retargeting mid-flight starts a NEW segment from the current
 *   rendered position and carries the current velocity (projected onto the
 *   new direction and capped), so the path bends instead of jerking and
 *   never swings back. There is no queue: the newest target replaces the old.
 *
 * Positions are normalized (0..1 of the guest frame); durations come from
 * the on-screen distance so a small preview and a full-window one both read
 * right.
 */
import type { Point } from "./mapping.js";

export const GLIDE = {
  /** Below this on-screen distance (px) the cursor simply lands. */
  snapPx: 2,
  /** Duration for the smallest real move (ms). */
  baseMs: 60,
  /** Extra time per doubling of the distance, measured in steps of `unitPx`. */
  perDoublingMs: 70,
  unitPx: 24,
  maxMs: 380,
  /** Reduced motion: moves land at once. */
  reducedMs: 0,
  /** Carried velocity is capped at this multiple of the new segment's mean speed. */
  carryLimit: 1.5,
} as const;

/** Drag travel keeps the real drag's pace, within these bounds (ms). */
export const DRAG_MIN_MS = 120;
export const DRAG_MAX_MS = 1600;
/** Getting to a drag's start point is quick: the drag itself is what matters. */
export const DRAG_APPROACH_MAX_MS = 200;

export function glideDuration(distancePx: number, reducedMotion: boolean): number {
  if (reducedMotion || !(distancePx >= GLIDE.snapPx)) return 0;
  const ms = GLIDE.baseMs + GLIDE.perDoublingMs * Math.log2(1 + distancePx / GLIDE.unitPx);
  return Math.min(GLIDE.maxMs, Math.round(ms));
}

export function dragDuration(realMs: number, reducedMotion: boolean): number {
  if (reducedMotion) return 0;
  const ms = Number.isFinite(realMs) ? realMs : DRAG_MIN_MS;
  return Math.min(DRAG_MAX_MS, Math.max(DRAG_MIN_MS, Math.round(ms)));
}

export interface Glide {
  readonly from: Point;
  readonly to: Point;
  /** Initial velocity, normalized units per ms. */
  readonly velocity: Point;
  readonly start: number;
  readonly duration: number;
}

export interface GlideSample {
  readonly position: Point;
  /** Normalized units per ms. */
  readonly velocity: Point;
  readonly done: boolean;
}

const ZERO: Point = { x: 0, y: 0 };

/**
 * A new segment toward `to`. `velocity` is what the cursor carries right
 * now; only its component along the new direction survives, capped, so a
 * reversal starts from rest instead of overshooting.
 */
export function startGlide(from: Point, to: Point, velocity: Point, start: number, duration: number): Glide {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (!(duration > 0) || length === 0) return { from: to, to, velocity: ZERO, start, duration: 0 };
  const ux = dx / length;
  const uy = dy / length;
  const along = velocity.x * ux + velocity.y * uy;
  const cap = (GLIDE.carryLimit * length) / duration;
  const carried = Math.min(Math.max(along, 0), cap);
  const carry = carried > 0 ? { x: ux * carried, y: uy * carried } : ZERO;
  return { from, to, velocity: carry, start, duration };
}

/** Position and velocity at `now` (cubic Hermite, end velocity 0). */
export function sampleGlide(glide: Glide, now: number): GlideSample {
  if (!(glide.duration > 0) || now >= glide.start + glide.duration) {
    return { position: glide.to, velocity: ZERO, done: true };
  }
  const T = glide.duration;
  const s = Math.max(0, (now - glide.start) / T);
  const s2 = s * s;
  const s3 = s2 * s;
  const h00 = 2 * s3 - 3 * s2 + 1;
  const h10 = s3 - 2 * s2 + s;
  const h01 = -2 * s3 + 3 * s2;
  const d00 = 6 * s2 - 6 * s;
  const d10 = 3 * s2 - 4 * s + 1;
  const d01 = -6 * s2 + 6 * s;
  const { from: p0, to: p1, velocity: v0 } = glide;
  return {
    position: {
      x: h00 * p0.x + h10 * T * v0.x + h01 * p1.x,
      y: h00 * p0.y + h10 * T * v0.y + h01 * p1.y,
    },
    velocity: {
      x: (d00 * p0.x + d10 * T * v0.x + d01 * p1.x) / T,
      y: (d00 * p0.y + d10 * T * v0.y + d01 * p1.y) / T,
    },
    done: false,
  };
}

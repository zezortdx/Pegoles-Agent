/**
 * Ported from bloub (https://github.com/jeremy-prt/bloub).
 * Copyright (c) 2026 Jérémy Perret. SPDX-License-Identifier: MIT
 * See LICENSE-bloub in this directory. Upstream: src/bot/eyefit.ts.
 * English names and comments adapted for Pegoles; geometry and timing retained.
 * Precomputed eye offsets. Solving once avoids jitter during gaze and shape interpolation.
 */
import { EXPRESSIONS, type BotExpression } from './expressions';
import { eyePoses } from './face';
import { radiusAtAngle, toPoints, type Point } from './shape';
import { SHAPES } from './skins';
import { STATES, type Pose, type StateDef, type StateId } from './states';
const R = 100;
const DRIFT_YAW = 5.5 + 1.6;
const DRIFT_PITCH = 4.2 + 1.3;
const DRIFT_X = 0.006;
const DRIFT_Y = 0.007;
interface Face {
  gaze: Pose['gaze'];
  split: number;
  eyes: Pose['eyes'];
}
interface Footprint {
  x: number;
  y: number;
  ax: number;
  ay: number;
  r: number;
  m: [
    number,
    number,
    number,
    number
  ];
}
function footprints(face: Face, sil: Pose['sil'], radii: number[]): Footprint[] {
  const out: Footprint[] = [];
  const poses = eyePoses(face.gaze, R, face.split);
  for (let i = 0; i < 2; i++) {
    const e = poses[i];
    if (e.depth <= 0.02)
      continue;
    const cfg = face.eyes[i];
    const phi = ((cfg.tilt ?? 0) * Math.PI) / 180;
    const cp = Math.cos(phi);
    const sp = Math.sin(phi);
    const ax = e.a * cp + e.c * sp;
    const ay = e.b * cp + e.d * sp;
    const cx = -e.a * sp + e.c * cp;
    const cy = -e.b * sp + e.d * cp;
    const hw = Math.max(cfg.w * R, 0.01) / 2;
    const hh = Math.max(cfg.h * R, 0.01) / 2;
    const r = Math.min(hw, hh);
    const long = hh > hw;
    const halfAxis = long ? hh - r : hw - r;
    const fit = radiusAtAngle(radii, Math.atan2(e.y, e.x) - sil.rot);
    out.push({
      x: e.x * fit,
      y: e.y * fit,
      ax: (long ? cx : ax) * halfAxis,
      ay: (long ? cy : ay) * halfAxis,
      r,
      m: [ax, ay, cx, cy]
    });
  }
  return out;
}
function closestApproach(pts: Point[], x0: number, y0: number, x1: number, y1: number) {
  const sx = x1 - x0;
  const sy = y1 - y0;
  const len2 = sx * sx + sy * sy;
  let best = Infinity;
  let vx = 0;
  let vy = 0;
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i];
    let t = len2 > 0 ? ((p.x - x0) * sx + (p.y - y0) * sy) / len2 : 0;
    t = t < 0 ? 0 : t > 1 ? 1 : t;
    const ex = x0 + t * sx - p.x;
    const ey = y0 + t * sy - p.y;
    const d2 = ex * ex + ey * ey;
    if (d2 < best) {
      best = d2;
      vx = ex;
      vy = ey;
    }
  }
  const d = Math.sqrt(best);
  return { d, ux: d > 1e-9 ? vx / d : 0, uy: d > 1e-9 ? vy / d : 0 };
}
interface Scenario {
  footprints: Footprint[];
  reference: Footprint[];
  contour: Point[];
  calContour: Point[];
}
const CENTER_DRIFT = Math.hypot(DRIFT_X, DRIFT_Y) * R;
function tightest(pts: Point[], emps: Footprint[], tx: number, ty: number) {
  let margin = Infinity;
  let ux = 0;
  let uy = 0;
  for (const e of emps) {
    const x = e.x + tx;
    const y = e.y + ty;
    const a = closestApproach(pts, x - e.ax, y - e.ay, x + e.ax, y + e.ay);
    const [m0, m1, m2, m3] = e.m;
    const radius = e.r * Math.hypot(m0 * a.ux + m1 * a.uy, m2 * a.ux + m3 * a.uy) + CENTER_DRIFT;
    if (a.d - radius < margin) {
      margin = a.d - radius;
      ux = a.ux;
      uy = a.uy;
    }
  }
  return { margin, ux, uy };
}
const DIRECTIONS = 12;
const BISECTION_STEPS = 8;
function solve(scenarios: Scenario[]): {
  x: number;
  y: number;
} {
  if (!scenarios.length)
    return { x: 0, y: 0 };
  const margin = (tx: number, ty: number) => {
    let m = Infinity;
    for (const ep of scenarios)
      m = Math.min(m, tightest(ep.contour, ep.footprints, tx, ty).margin);
    return m;
  };
  let required = Infinity;
  for (const ep of scenarios) {
    required = Math.min(required, tightest(ep.calContour, ep.reference, 0, 0).margin);
  }
  let mx = 0;
  let my = 0;
  const emps = scenarios[0].footprints;
  for (const e of emps) {
    mx -= e.x / emps.length;
    my -= e.y / emps.length;
  }
  const maxTravel = Math.max(0.35 * R, Math.hypot(mx, my) * 1.25);
  required = Math.min(required, margin(mx, my));
  const initial = margin(0, 0);
  if (initial >= required && initial >= 0)
    return { x: 0, y: 0 };
  const target = Math.max(required, 0);
  let bestX = 0;
  let bestY = 0;
  let bestDistance = Infinity;
  let fallbackX = 0;
  let fallbackY = 0;
  let fallbackMargin = initial;
  for (let d = 0; d < DIRECTIONS; d++) {
    const a = (d / DIRECTIONS) * Math.PI * 2;
    const ux = Math.cos(a);
    const uy = Math.sin(a);
    if (margin(ux * maxTravel, uy * maxTravel) < target) {
      for (const k of [0.3, 0.6, 1]) {
        const m = margin(ux * maxTravel * k, uy * maxTravel * k);
        if (m > fallbackMargin) {
          fallbackMargin = m;
          fallbackX = ux * maxTravel * k;
          fallbackY = uy * maxTravel * k;
        }
      }
      continue;
    }
    let low = 0;
    let high = maxTravel;
    for (let i = 0; i < BISECTION_STEPS; i++) {
      const mid = (low + high) / 2;
      if (margin(ux * mid, uy * mid) >= target)
        high = mid;
      else
        low = mid;
    }
    if (high < bestDistance) {
      bestDistance = high;
      bestX = ux * high;
      bestY = uy * high;
    }
  }
  const x = bestDistance === Infinity ? fallbackX : bestX;
  const y = bestDistance === Infinity ? fallbackY : bestY;
  return { x: +(x / R).toFixed(6), y: +(y / R).toFixed(6) };
}
function faceFor(def: StateDef, pose: Pose, expr: BotExpression | null): Face {
  if (def.baseFace && expr)
    return { gaze: expr.gaze, split: expr.split, eyes: expr.eyes };
  return { gaze: pose.gaze, split: pose.split, eyes: pose.eyes };
}
function sampleTimes(def: StateDef): number[] {
  const signature = (p: Pose) => JSON.stringify([p.gaze, p.split, p.eyes, p.sil.rot, p.sil.cx, p.sil.cy, p.sil.sx, p.sil.sy]);
  if (signature(def.pose(0)) === signature(def.pose(def.duration)))
    return [0];
  const n = 3;
  return Array.from({ length: n }, (_, i) => (i / (n - 1)) * def.duration);
}
function offsetFor(def: StateDef, radii: number[], expr: BotExpression | null): {
  x: number;
  y: number;
} {
  const scenarios: Scenario[] = [];
  for (const t of sampleTimes(def)) {
    const pose = def.pose(t);
    const contour = toPoints({ ...pose.sil, radii }, R);
    const calContour = toPoints(pose.sil, R);
    const v = faceFor(def, pose, expr);
    const corners: Face[] = [];
    for (const dy of [-DRIFT_YAW, DRIFT_YAW]) {
      for (const dp of [-DRIFT_PITCH, DRIFT_PITCH]) {
        corners.push({
          ...v,
          gaze: { yaw: v.gaze.yaw + dy, pitch: v.gaze.pitch + dp, roll: v.gaze.roll }
        });
      }
    }
    for (const c of corners) {
      scenarios.push({
        footprints: footprints(c, pose.sil, radii),
        reference: footprints(c, pose.sil, pose.sil.radii),
        contour,
        calContour
      });
    }
  }
  return solve(scenarios);
}
const ZERO_OFFSET = { x: 0, y: 0 } as const;
const offsetKey = (state: StateId, expr: string | null) => `${state}|${expr ?? ''}`;
function buildOffsets(): Map<number[], Map<string, {
  x: number;
  y: number;
}>> {
  return new Map(SHAPES.map((shape) => {
    const table = new Map<string, {
      x: number;
      y: number;
    }>();
    for (const def of STATES) {
      if (!def.baseBody)
        continue;
      const expressions = def.baseFace ? [null, ...EXPRESSIONS] : [null];
      for (const expr of expressions) {
        table.set(offsetKey(def.id, expr?.id ?? null), offsetFor(def, shape.radii, expr));
      }
    }
    return [shape.radii, table];
  }));
}
const OFFSETS = buildOffsets();
export function eyeOffset(radii: number[] | null, state: StateId, expr: string | null): {
  x: number;
  y: number;
} {
  if (!radii)
    return ZERO_OFFSET;
  const table = OFFSETS.get(radii);
  if (!table)
    return ZERO_OFFSET;
  return table.get(offsetKey(state, expr)) ?? table.get(offsetKey(state, null)) ?? ZERO_OFFSET;
}
export const EYE_FIT_TESTS = { buildOffsets };

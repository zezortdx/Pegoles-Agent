/**
 * Ported from bloub (https://github.com/jeremy-prt/bloub).
 * Copyright (c) 2026 Jérémy Perret. SPDX-License-Identifier: MIT
 * See LICENSE-bloub in this directory. Upstream: src/ui/gaze.ts.
 * English names and comments adapted for Pegoles; geometry and timing retained.
 * Pure pointer-to-gaze targets in normalized screen coordinates.
 */
import type { Look } from './engine';
import type { ExpressionId } from './expressions';
import { clamp, easings } from './math';
export const YAW_MAX = 16;
export const PITCH_MAX = 13;
export const PITCH = 10;
export const TURN = 26;
export const SPIN = 360;
export const TURN_TIME = 1.1;
export const MOODS: readonly ExpressionId[] = [
  "surprised",
  "happy",
  "laughing",
  "excited",
  "proud",
  "bored"
];
export type GazeScript = (t: number) => Look;
export const SPIN_TIME = 1.5;
export const spinLook: GazeScript = (t) => ({
  yaw: 0,
  pitch: 0,
  mix: 0,
  spin: SPIN * (1 - easings.easeInOutCubic(clamp(t / SPIN_TIME))),
  wander: 1
});
export interface Aim {
  nx: number;
  ny: number;
  arrival: number;
  pointer: boolean;
}
export function lookTarget({ nx, ny, arrival, pointer }: Aim): Look {
  return {
    yaw: -TURN + nx * YAW_MAX,
    pitch: PITCH - ny * PITCH_MAX,
    mix: arrival,
    spin: SPIN * (1 - arrival),
    wander: pointer ? 0 : 1
  };
}

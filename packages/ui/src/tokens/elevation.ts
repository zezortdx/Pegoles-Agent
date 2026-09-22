/**
 * Pegoles Flux Glass — elevation.
 *
 * On a near-black canvas shadows barely read, so depth is carried mostly
 * by lightness (surface → elevated → raised) and the specular top edge;
 * shadows only separate FLOATING layers from what scrolls beneath them.
 * Shadow color is the void itself (never pure black).
 */
import { resolveChannels, semantic } from "./semantic.js";

export interface ShadowLayer {
  readonly y: number;
  readonly blur: number;
  readonly spread: number;
  /** Alpha of `shadow.color`. */
  readonly alpha: number;
}

export const elevation = {
  0: [],
  /** Controls resting on a surface. */
  1: [{ y: 1, blur: 2, spread: 0, alpha: 0.5 }],
  /** Floating surfaces (cards over content, rails). */
  2: [
    { y: 1, blur: 2, spread: 0, alpha: 0.55 },
    { y: 10, blur: 28, spread: -8, alpha: 0.7 },
  ],
  /** Top-most floating layer (CommandBar, TaskController, popovers). */
  3: [
    { y: 2, blur: 6, spread: 0, alpha: 0.5 },
    { y: 24, blur: 60, spread: -14, alpha: 0.85 },
  ],
} as const satisfies Record<number, readonly ShadowLayer[]>;

export type ElevationLevel = keyof typeof elevation;

export function shadowCss(level: ElevationLevel): string {
  const layers: readonly ShadowLayer[] = elevation[level];
  if (layers.length === 0) return "none";
  const channels = resolveChannels(semantic.shadow.color);
  return layers
    .map((l) => `0 ${l.y}px ${l.blur}px ${l.spread}px rgb(${channels} / ${l.alpha})`)
    .join(", ");
}

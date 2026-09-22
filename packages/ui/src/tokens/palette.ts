/**
 * Pegoles Flux Glass — raw palette (Phase 4 source of truth).
 *
 * Raw colors are referenced ONLY by semantic tokens (see ./semantic.ts).
 * Components never import this file directly. Most of the UI stays near
 * black; blue means Pegoles presence, agent activity, focus, energy.
 */
export const palette = {
  void: "#02040A",
  surface: "#070B14",
  blueDeep: "#002A8E",
  blueShadow: "#0139C2",
  pegolesBlue: "#015FF8",
  electric: "#169CFD",
  cyan: "#4CCEFC",
  ice: "#97EBFD",
  text: "#F4F7FC",
  muted: "#8B96A8",
} as const;

export type PaletteToken = keyof typeof palette;

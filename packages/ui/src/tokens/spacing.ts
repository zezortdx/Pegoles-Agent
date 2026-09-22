/**
 * Pegoles Flux Glass — spacing (px).
 *
 * A 4px grid, but used as RHYTHM, not as uniform padding: tight inside a
 * group, generous between groups, very generous between sections. Reach
 * for `rhythm` (intent) before `space` (raw step).
 */
export const space = {
  none: 0,
  hair: 1,
  xxs: 2,
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 24,
  xxl: 32,
  x3: 48,
  x4: 64,
  x5: 96,
} as const;

export type SpaceToken = keyof typeof space;

export const rhythm = {
  /** Icon ↔ label inside one control. */
  inline: 6,
  /** Label ↔ value, title ↔ subtitle: parts of one thing. */
  stackTight: 4,
  /** Rows within one group. */
  stack: 8,
  /** Horizontal padding inside controls. */
  controlX: 14,
  /** Padding inside compact surfaces (chips, rows). */
  surfaceInset: 16,
  /** Padding inside panels (task controller, viewport chrome). */
  panelInset: 20,
  /** Between groups inside one surface. */
  group: 24,
  /** Between page sections. */
  section: 72,
  /** Page gutter. */
  page: 32,
} as const;

export type RhythmToken = keyof typeof rhythm;

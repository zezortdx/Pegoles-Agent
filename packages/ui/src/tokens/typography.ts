/**
 * Pegoles Flux Glass — typography.
 *
 * Pairing: the platform system face for all UI (SF Pro via -apple-system
 * on macOS, Segoe UI Variable on Windows) + the platform monospace for
 * technical data (SF Mono / Cascadia Mono). Pegoles is an operating
 * environment that sits next to native traffic lights and native Liquid
 * Glass controls, so the UI speaks the system's typographic voice, with
 * its optical sizes and tracking tables, at zero bytes. The mono face
 * carries identity: labels, timings, dimensions and IDs read as
 * instrument readouts. No web fonts are bundled (see DESIGN_SYSTEM.md).
 */
export const fontFamily = {
  sans: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
  mono: 'ui-monospace, "SF Mono", SFMono-Regular, "Cascadia Mono", "Segoe UI Mono", Menlo, Consolas, monospace',
} as const;

export type FontFamilyToken = keyof typeof fontFamily;

/** Three weights only. */
export const fontWeight = {
  regular: 400,
  medium: 500,
  semibold: 600,
} as const;

export interface TypeRole {
  readonly family: FontFamilyToken;
  /** px */
  readonly size: number;
  /** px */
  readonly lineHeight: number;
  readonly weight: (typeof fontWeight)[keyof typeof fontWeight];
  /** em — negative as size grows, positive for small caps labels. */
  readonly tracking: number;
  readonly uppercase?: boolean;
  readonly tabular?: boolean;
}

export const typeScale = {
  display: { family: "sans", size: 34, lineHeight: 40, weight: 600, tracking: -0.024 },
  title1: { family: "sans", size: 26, lineHeight: 32, weight: 600, tracking: -0.02 },
  title2: { family: "sans", size: 19, lineHeight: 25, weight: 600, tracking: -0.012 },
  headline: { family: "sans", size: 15, lineHeight: 21, weight: 600, tracking: -0.008 },
  body: { family: "sans", size: 14, lineHeight: 21, weight: 400, tracking: -0.006 },
  callout: { family: "sans", size: 13, lineHeight: 19, weight: 400, tracking: -0.003 },
  footnote: { family: "sans", size: 12, lineHeight: 17, weight: 400, tracking: 0 },
  caption: { family: "sans", size: 11, lineHeight: 15, weight: 500, tracking: 0.01 },
  /** Instrument label: small mono caps. */
  label: {
    family: "mono",
    size: 10.5,
    lineHeight: 14,
    weight: 500,
    tracking: 0.08,
    uppercase: true,
  },
  /** Technical data: timings, sizes, IDs. Tabular so columns align. */
  data: { family: "mono", size: 12, lineHeight: 16, weight: 400, tracking: 0, tabular: true },
} as const satisfies Record<string, TypeRole>;

export type TypeRoleName = keyof typeof typeScale;

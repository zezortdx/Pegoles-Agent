/**
 * Pegoles Flux Glass — effects tiers.
 *
 * Three tiers trade GPU cost for richness. All three must feel like
 * Pegoles: the difference is blur depth, glow intensity, ambient motion
 * and transition richness — never information. The Rust ResourceGovernor
 * (`recommend_effects`, pegoles-computer) recommends a tier; the user can
 * override it; reduced motion is an independent axis (see DESIGN_SYSTEM.md).
 */

export const EFFECTS_TIERS = ["full", "reduced", "minimal"] as const;
export type EffectsTier = (typeof EFFECTS_TIERS)[number];

export type GlassMaterial = "regular" | "clear" | "electric";
export const GLASS_MATERIALS: readonly GlassMaterial[] = ["regular", "clear", "electric"];

export interface GlassTierRecipe {
  /** Backdrop blur radius in px; 0 = no backdrop-filter at all. */
  readonly blurPx: number;
  /**
   * Opacity of the material tint — the readability scrim. Higher where
   * blur is weaker (sharper backdrops need more scrim). Verified against
   * worst-case backdrops by tests (materials.test.ts).
   */
  readonly alpha: number;
}

/**
 * How state changes animate:
 * - `rich`: springs, shared-layout morphs, blur-materialize on glass.
 * - `standard`: springs and shared-layout morphs, no blur-materialize.
 * - `simple`: short crossfades only (no layout morphs).
 */
export type TransitionRichness = "rich" | "standard" | "simple";

export interface EffectsTierParams {
  readonly tier: EffectsTier;
  readonly label: string;
  readonly summary: string;
  readonly glass: Readonly<Record<GlassMaterial, GlassTierRecipe>>;
  /** Backdrop saturation boost (1 = none). */
  readonly saturate: number;
  /** Multiplier for glows and energy edges (0..1). */
  readonly glowIntensity: number;
  /** Opacity of the specular edge highlight (0..1). Static, never animated. */
  readonly specular: number;
  readonly ambient: {
    /** Whether small ambient loops (presence breathing) may run at all. */
    readonly enabled: boolean;
    /** Loop period in ms (within the 4–8 s ambient window). */
    readonly periodMs: number;
    /** Relative amplitude of ambient loops (0..1). */
    readonly amplitude: number;
  };
  readonly transition: TransitionRichness;
  /**
   * Blur budget: max backdrop-filter surfaces visible at once. The dev
   * blur audit (design lab) flags anything above it.
   */
  readonly maxBackdropSurfaces: number;
}

export const effectsTiers: Readonly<Record<EffectsTier, EffectsTierParams>> = {
  full: {
    tier: "full",
    label: "Full",
    summary: "Deep glass, living presence, rich spatial transitions.",
    glass: {
      regular: { blurPx: 28, alpha: 0.82 },
      clear: { blurPx: 18, alpha: 0.6 },
      electric: { blurPx: 28, alpha: 0.9 },
    },
    saturate: 1.6,
    glowIntensity: 1,
    specular: 1,
    ambient: { enabled: true, periodMs: 6000, amplitude: 1 },
    transition: "rich",
    maxBackdropSurfaces: 6,
  },
  reduced: {
    tier: "reduced",
    label: "Reduced",
    summary: "Lighter blur, calmer glow, a single slow ambient breath.",
    glass: {
      regular: { blurPx: 14, alpha: 0.86 },
      clear: { blurPx: 10, alpha: 0.66 },
      electric: { blurPx: 14, alpha: 0.92 },
    },
    saturate: 1.3,
    glowIntensity: 0.7,
    specular: 0.85,
    ambient: { enabled: true, periodMs: 8000, amplitude: 0.5 },
    transition: "standard",
    maxBackdropSurfaces: 3,
  },
  minimal: {
    tier: "minimal",
    label: "Minimal",
    summary: "Solid translucent surfaces, crisp strokes, no continuous motion.",
    glass: {
      regular: { blurPx: 0, alpha: 0.94 },
      clear: { blurPx: 0, alpha: 0.9 },
      electric: { blurPx: 0, alpha: 0.96 },
    },
    saturate: 1,
    glowIntensity: 0.4,
    specular: 0.7,
    ambient: { enabled: false, periodMs: 8000, amplitude: 0 },
    transition: "simple",
    maxBackdropSurfaces: 0,
  },
};

export function isEffectsTier(value: unknown): value is EffectsTier {
  return typeof value === "string" && (EFFECTS_TIERS as readonly string[]).includes(value);
}

export interface EffectsTierInput {
  /** From the Rust ResourceGovernor; null/undefined when not known yet. */
  readonly recommended?: EffectsTier | null;
  /** Explicit user choice in Settings; "auto"/null/undefined = follow recommendation. */
  readonly userOverride?: EffectsTier | "auto" | null;
  /** OS reduced-motion preference. Read-only input: never altered here. */
  readonly prefersReducedMotion: boolean;
  /** OS Low Power Mode / battery saver. */
  readonly lowPower?: boolean;
}

export type EffectsTierSource = "user" | "governor" | "default";

export interface EffectsSelection {
  readonly tier: EffectsTier;
  /** Mirrors the input exactly — tiers never change accessibility settings. */
  readonly reducedMotion: boolean;
  readonly source: EffectsTierSource;
  /** True when Low Power capped a Full recommendation to Reduced. */
  readonly lowPowerApplied: boolean;
}

/** Used until the governor answers: premium but cheap, and never a downgrade flash. */
export const DEFAULT_EFFECTS_TIER: EffectsTier = "reduced";

/**
 * Pure tier selection.
 * 1. A user override always wins (even on Low Power: it was explicit).
 * 2. Otherwise the governor recommendation, else DEFAULT_EFFECTS_TIER.
 * 3. Low Power caps Full → Reduced (never touches Reduced/Minimal).
 * Reduced motion is orthogonal: it passes through untouched and never
 * lowers the tier — it changes HOW things move, not how rich they look.
 */
export function selectEffectsTier(input: EffectsTierInput): EffectsSelection {
  const override = input.userOverride;
  if (override !== undefined && override !== null && override !== "auto") {
    return {
      tier: override,
      reducedMotion: input.prefersReducedMotion,
      source: "user",
      lowPowerApplied: false,
    };
  }
  const recommended = input.recommended ?? null;
  const base: EffectsTier = recommended ?? DEFAULT_EFFECTS_TIER;
  const capped = input.lowPower === true && base === "full";
  return {
    tier: capped ? "reduced" : base,
    reducedMotion: input.prefersReducedMotion,
    source: recommended === null ? "default" : "governor",
    lowPowerApplied: capped,
  };
}

export type TransitionStyle = "morph" | "crossfade";

/**
 * Spatial morphs need a tier that allows them AND no reduced-motion
 * preference; otherwise state changes crossfade (state is still
 * communicated — only the movement is removed).
 */
export function resolveTransitionStyle(tier: EffectsTier, reducedMotion: boolean): TransitionStyle {
  if (reducedMotion) return "crossfade";
  return effectsTiers[tier].transition === "simple" ? "crossfade" : "morph";
}

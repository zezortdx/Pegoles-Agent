/**
 * Pegoles Flux Glass — glass materials.
 *
 * A material = tint (semantic) × per-tier recipe (blur + scrim opacity)
 * × edge treatment. The CSS backend (`GlassSurface`) is the only glass on
 * web surfaces on every OS; native Liquid Glass is reserved for native
 * overlays above the VM framebuffer (see DESIGN_SYSTEM.md "GlassBackend").
 */
import { hexToRgb, type Rgb } from "./color.js";
import { effectsTiers, type EffectsTier, type GlassMaterial } from "./effects.js";
import { palette } from "./palette.js";
import { resolveColor, semantic, type ColorToken } from "./semantic.js";

export interface GlassMaterialSpec {
  readonly material: GlassMaterial;
  readonly label: string;
  readonly use: string;
  readonly tint: ColorToken;
  /** Flat layer used when this material is nested inside another glass. */
  readonly nestedTint: ColorToken;
  /** Which secondary text color is legible on it (vibrant on clear). */
  readonly secondaryText: ColorToken;
  readonly edge: ColorToken;
}

export const glassMaterials: Readonly<Record<GlassMaterial, GlassMaterialSpec>> = {
  regular: {
    material: "regular",
    label: "Regular",
    use: "Floating chrome over moving content: toolbars, the CommandBar, menus.",
    tint: semantic.glass.regular,
    nestedTint: semantic.border.subtle,
    secondaryText: semantic.text.secondary,
    edge: semantic.border.default,
  },
  clear: {
    material: "clear",
    label: "Clear",
    use: "Small, high-signal chips over rich backdrops. Primary-weight content only.",
    tint: semantic.glass.clear,
    nestedTint: semantic.border.subtle,
    secondaryText: semantic.text.vibrantSecondary,
    edge: semantic.border.strong,
  },
  electric: {
    material: "electric",
    label: "Electric",
    use: "Pegoles is present and working: TaskController, focused command surface.",
    tint: semantic.glass.electric,
    nestedTint: semantic.accent.soft,
    secondaryText: semantic.text.secondary,
    edge: semantic.stroke.electric,
  },
};

/**
 * Worst-case backdrops the contrast guarantees are proven against.
 * Glass never sits over the VM framebuffer (the native view is above the
 * webview), so the brightest things that can pass behind web glass are
 * Pegoles' own lights: Cyan (glow cores) and Ice (the mark's eyes).
 */
export const WORST_CASE_BACKDROP: Readonly<{ primaryText: Rgb; secondaryText: Rgb }> = {
  /** Primary text must stay ≥ 4.5:1 even over Ice. */
  primaryText: hexToRgb(palette.ice),
  /** Secondary text must stay ≥ 4.5:1 over Cyan. */
  secondaryText: hexToRgb(palette.cyan),
};

export interface GlassCss {
  readonly background: string;
  readonly backdropFilter: string;
}

export function glassCss(tier: EffectsTier, material: GlassMaterial): GlassCss {
  const params = effectsTiers[tier];
  const recipe = params.glass[material];
  const tint = resolveColor({ ...glassMaterials[material].tint, alpha: recipe.alpha });
  const backdropFilter =
    recipe.blurPx > 0
      ? `blur(${recipe.blurPx}px) saturate(${Math.round(params.saturate * 100)}%)`
      : "none";
  return { background: tint.css, backdropFilter };
}

export function materialUsesBackdrop(tier: EffectsTier, material: GlassMaterial): boolean {
  return effectsTiers[tier].glass[material].blurPx > 0;
}

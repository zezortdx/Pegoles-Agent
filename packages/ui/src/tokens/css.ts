/**
 * Pegoles Flux Glass — CSS custom property emission.
 *
 * The TypeScript tokens are the single source of truth; CSS never
 * re-declares a value. `fluxGlassStyleSheet()` renders every token as a
 * custom property on :root plus one block per effects tier keyed on
 * `[data-effects-tier]`, so switching tiers is one attribute write (no
 * React rerender) and a subtree can be scoped to a different tier.
 * `FluxGlassRoot` injects this sheet once at startup.
 */
import { effectsTiers, EFFECTS_TIERS, GLASS_MATERIALS, type EffectsTier } from "./effects.js";
import { elevation, shadowCss, type ElevationLevel } from "./elevation.js";
import { glassCss, glassMaterials } from "./materials.js";
import { duration, easing } from "./motion.js";
import { radius, radiusRole } from "./radius.js";
import { resolveChannels, resolveColor, semantic, semanticEntries } from "./semantic.js";
import { rhythm, space } from "./spacing.js";
import { fontFamily, typeScale, type TypeRole } from "./typography.js";

export type CssVariables = Readonly<Record<`--pg-${string}`, string>>;

const GROUP_ALIAS: Readonly<Record<string, string>> = {
  background: "bg",
  statusSoft: "status-soft",
  glass: "glass-tint",
};

export function kebab(name: string): string {
  return name.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

/** CSS variable name of a semantic color path, e.g. `text.secondary` → `--pg-text-secondary`. */
export function semanticVar(path: string): `--pg-${string}` {
  const [group = "", name = ""] = path.split(".");
  const prefix = GROUP_ALIAS[group] ?? kebab(group);
  return `--pg-${prefix}-${kebab(name)}`;
}

/** Tier-independent variables. */
export function cssVariables(): CssVariables {
  const vars: Record<`--pg-${string}`, string> = {};

  for (const { path, token } of semanticEntries()) {
    const name = semanticVar(path);
    vars[name] = resolveColor(token).css;
    vars[`${name}-rgb`] = resolveChannels(token);
  }

  for (const [key, value] of Object.entries(space)) vars[`--pg-space-${key}`] = `${value}px`;
  for (const [key, value] of Object.entries(rhythm)) vars[`--pg-rhythm-${kebab(key)}`] = `${value}px`;
  for (const [key, value] of Object.entries(radius)) vars[`--pg-radius-${key}`] = `${value}px`;
  for (const [key, value] of Object.entries(radiusRole)) vars[`--pg-radius-${kebab(key)}`] = `${value}px`;

  vars["--pg-font-sans"] = fontFamily.sans;
  vars["--pg-font-mono"] = fontFamily.mono;
  for (const [role, spec] of Object.entries(typeScale) as [string, TypeRole][]) {
    const r = kebab(role);
    vars[`--pg-type-${r}-size`] = `${spec.size}px`;
    vars[`--pg-type-${r}-line`] = `${spec.lineHeight}px`;
    vars[`--pg-type-${r}-weight`] = String(spec.weight);
    vars[`--pg-type-${r}-tracking`] = `${spec.tracking}em`;
  }

  for (const [key, value] of Object.entries(duration)) vars[`--pg-duration-${kebab(key)}`] = `${value}ms`;
  for (const [key, value] of Object.entries(easing)) vars[`--pg-ease-${kebab(key)}`] = value;

  for (const level of Object.keys(elevation)) {
    vars[`--pg-shadow-${level}`] = shadowCss(Number(level) as ElevationLevel);
  }

  for (const material of GLASS_MATERIALS) {
    const spec = glassMaterials[material];
    vars[`--pg-glass-edge-${material}`] = resolveColor(spec.edge).css;
    vars[`--pg-glass-nested-${material}`] = resolveColor(spec.nestedTint).css;
    vars[`--pg-glass-secondary-${material}`] = resolveColor(spec.secondaryText).css;
  }

  return vars;
}

/** Variables that change with the effects tier. */
export function tierCssVariables(tier: EffectsTier): CssVariables {
  const params = effectsTiers[tier];
  const vars: Record<`--pg-${string}`, string> = {};
  for (const material of GLASS_MATERIALS) {
    const css = glassCss(tier, material);
    vars[`--pg-glass-bg-${material}`] = css.background;
    vars[`--pg-glass-filter-${material}`] = css.backdropFilter;
  }
  vars["--pg-glow"] = String(params.glowIntensity);
  vars["--pg-specular"] = String(params.specular);
  vars["--pg-ambient-period"] = `${params.ambient.periodMs}ms`;
  vars["--pg-ambient-amplitude"] = String(params.ambient.amplitude);
  vars["--pg-cursor-trail"] = String(params.cursorTrailLength);
  return vars;
}

function block(selector: string, vars: CssVariables): string {
  const body = Object.entries(vars)
    .map(([name, value]) => `  ${name}: ${value};`)
    .join("\n");
  return `${selector} {\n${body}\n}`;
}

function typeClass(role: string, spec: TypeRole): string {
  const r = kebab(role);
  const lines = [
    `  font-family: var(--pg-font-${spec.family});`,
    `  font-size: var(--pg-type-${r}-size);`,
    `  line-height: var(--pg-type-${r}-line);`,
    `  font-weight: var(--pg-type-${r}-weight);`,
    `  letter-spacing: var(--pg-type-${r}-tracking);`,
  ];
  if (spec.uppercase === true) lines.push("  text-transform: uppercase;");
  if (spec.tabular === true) lines.push("  font-variant-numeric: tabular-nums;");
  return `.pg-type-${r} {\n${lines.join("\n")}\n}`;
}

/** Accessibility fallbacks that never depend on the tier. */
function accessibilityBlocks(): string {
  const solid: Record<`--pg-${string}`, string> = {};
  for (const material of GLASS_MATERIALS) {
    solid[`--pg-glass-bg-${material}`] = resolveColor({ ...glassMaterials[material].tint, alpha: 0.97 }).css;
    solid[`--pg-glass-filter-${material}`] = "none";
  }
  const contrast: Record<`--pg-${string}`, string> = {
    [semanticVar("border.subtle")]: resolveColor({ ...semantic.border.subtle, alpha: 0.16 }).css,
    [semanticVar("border.default")]: resolveColor({ ...semantic.border.default, alpha: 0.28 }).css,
    [semanticVar("border.strong")]: resolveColor({ ...semantic.border.strong, alpha: 0.42 }).css,
  };
  return [
    `@media (prefers-reduced-transparency: reduce) {\n${block(":root, [data-effects-tier]", solid)}\n}`,
    `@media (prefers-contrast: more) {\n${block(":root, [data-effects-tier]", contrast)}\n}`,
  ].join("\n");
}

/** The complete token stylesheet (deterministic; covered by tests). */
export function fluxGlassStyleSheet(): string {
  const parts: string[] = [];
  parts.push(
    block(":root", {
      ...cssVariables(),
      // Outside any tier scope, fall back to the default (Reduced) recipe.
      ...tierCssVariables("reduced"),
    }).replace(":root {\n", ":root {\n  color-scheme: dark;\n"),
  );
  for (const tier of EFFECTS_TIERS) {
    parts.push(block(`[data-effects-tier="${tier}"]`, tierCssVariables(tier)));
  }
  parts.push(accessibilityBlocks());
  for (const [role, spec] of Object.entries(typeScale) as [string, TypeRole][]) {
    parts.push(typeClass(role, spec));
  }
  return `${parts.join("\n")}\n`;
}

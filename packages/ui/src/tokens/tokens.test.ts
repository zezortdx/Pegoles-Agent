import { describe, expect, it } from "vitest";
import { composite, contrastRatio, hexToRgb, isBlue, mixRgb, type Rgb } from "./color.js";
import { cssVariables, fluxGlassStyleSheet, semanticVar, tierCssVariables } from "./css.js";
import { EFFECTS_TIERS, effectsTiers, GLASS_MATERIALS } from "./effects.js";
import { glassMaterials, WORST_CASE_BACKDROP } from "./materials.js";
import { duration } from "./motion.js";
import { palette } from "./palette.js";
import { rawHex, resolveColor, semantic, semanticEntries } from "./semantic.js";
import { signal } from "./signal.js";

const rgb = (hex: string): Rgb => hexToRgb(hex);

describe("seed tokens", () => {
  it("keeps the exact Phase 4 palette", () => {
    expect(palette).toEqual({
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
    });
  });

  it("keeps the seeded motion durations", () => {
    expect(duration).toMatchObject({
      instant: 100,
      interaction: 180,
      standard: 260,
      fluid: 420,
      scene: 650,
      ambientMin: 4000,
      ambientMax: 8000,
    });
  });
});

describe("semantic tokens", () => {
  it("resolve only to raw palette/signal colors (optionally mixed or with alpha)", () => {
    for (const { path, token } of semanticEntries()) {
      const base = rawHex(token.base);
      expect(Object.values({ ...palette, ...signal }), path).toContain(base);
      if (token.mix) {
        expect(Object.values(palette), path).toContain(rawHex(token.mix.toward));
        expect(token.mix.amount, path).toBeGreaterThan(0);
        expect(token.mix.amount, path).toBeLessThan(1);
      }
      const resolved = resolveColor(token);
      const expected = token.mix ? mixRgb(rgb(base), rgb(rawHex(token.mix.toward)), token.mix.amount) : rgb(base);
      expect(resolved.rgb, path).toEqual(expected);
      expect(token.alpha, path).toBeGreaterThan(0);
      expect(token.alpha, path).toBeLessThanOrEqual(1);
    }
  });

  it("maps the core roles to the exact palette values", () => {
    expect(resolveColor(semantic.background.primary).css).toBe(palette.void);
    expect(resolveColor(semantic.background.secondary).css).toBe(palette.surface);
    expect(resolveColor(semantic.surface.default).css).toBe(palette.surface);
    expect(resolveColor(semantic.text.primary).css).toBe(palette.text);
    expect(resolveColor(semantic.text.secondary).css).toBe(palette.muted);
    expect(resolveColor(semantic.accent.primary).css).toBe(palette.pegolesBlue);
    expect(resolveColor(semantic.accent.active).css).toBe(palette.electric);
    expect(resolveColor(semantic.focus.ring).css).toBe(palette.cyan);
  });

  it("never uses blue for a status color", () => {
    for (const [name, token] of Object.entries(semantic.status)) {
      expect(isBlue(resolveColor(token).rgb), `status.${name}`).toBe(false);
    }
    for (const [name, token] of Object.entries(semantic.statusSoft)) {
      expect(isBlue(resolveColor(token).rgb), `statusSoft.${name}`).toBe(false);
    }
  });

  it("recognises the brand blues as blue (sanity check of the detector)", () => {
    for (const key of ["pegolesBlue", "electric", "cyan", "blueDeep", "blueShadow"] as const) {
      expect(isBlue(rgb(palette[key])), key).toBe(true);
    }
  });

  it("status colors are distinguishable on dark surfaces (≥ 3:1 non-text, ≥ 4.5:1 as text)", () => {
    const backgrounds = [
      resolveColor(semantic.background.primary).rgb,
      resolveColor(semantic.surface.default).rgb,
      resolveColor(semantic.surface.elevated).rgb,
    ];
    for (const [name, token] of Object.entries(semantic.status)) {
      for (const bg of backgrounds) {
        expect(contrastRatio(resolveColor(token).rgb, bg), `status.${name}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("text passes WCAG AA on every opaque surface", () => {
    const surfaces = [
      semantic.background.primary,
      semantic.background.secondary,
      semantic.surface.elevated,
      semantic.surface.raised,
    ].map((t) => resolveColor(t).rgb);
    for (const bg of surfaces) {
      expect(contrastRatio(resolveColor(semantic.text.primary).rgb, bg)).toBeGreaterThanOrEqual(7);
      expect(contrastRatio(resolveColor(semantic.text.secondary).rgb, bg)).toBeGreaterThanOrEqual(4.5);
      expect(contrastRatio(resolveColor(semantic.text.accent).rgb, bg)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("white label on the primary accent fill passes AA", () => {
    const fill = resolveColor(semantic.accent.primary).rgb;
    expect(contrastRatio(resolveColor(semantic.text.onAccent).rgb, fill)).toBeGreaterThanOrEqual(4.5);
  });

  it("focus ring is clearly visible against the canvas (≥ 3:1)", () => {
    const ring = resolveColor(semantic.focus.ring).rgb;
    expect(contrastRatio(ring, resolveColor(semantic.background.primary).rgb)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(ring, resolveColor(semantic.surface.elevated).rgb)).toBeGreaterThanOrEqual(3);
  });
});

describe("glass readability (scrim per tier)", () => {
  for (const tier of EFFECTS_TIERS) {
    for (const material of GLASS_MATERIALS) {
      it(`${tier}/${material}: primary text ≥ 4.5:1 over Ice, secondary ≥ 4.5:1 over Cyan`, () => {
        const spec = glassMaterials[material];
        const recipe = effectsTiers[tier].glass[material];
        const tint = resolveColor(spec.tint).rgb;
        const primary = resolveColor(semantic.text.primary).rgb;
        const secondary = resolveColor(spec.secondaryText).rgb;
        const overIce = composite(tint, recipe.alpha, WORST_CASE_BACKDROP.primaryText);
        const overCyan = composite(tint, recipe.alpha, WORST_CASE_BACKDROP.secondaryText);
        expect(contrastRatio(primary, overIce)).toBeGreaterThanOrEqual(4.5);
        expect(contrastRatio(secondary, overCyan)).toBeGreaterThanOrEqual(4.5);
      });
    }
  }
});

describe("CSS variable emission", () => {
  it("emits every semantic color with its channels", () => {
    const vars = cssVariables();
    for (const { path, token } of semanticEntries()) {
      const name = semanticVar(path);
      expect(vars[name], name).toBe(resolveColor(token).css);
      expect(vars[`${name}-rgb`], `${name}-rgb`).toMatch(/^\d+ \d+ \d+$/);
    }
    expect(vars["--pg-bg-primary"]).toBe("#02040A");
    expect(vars["--pg-text-secondary"]).toBe("#8B96A8");
    expect(vars["--pg-duration-interaction"]).toBe("180ms");
    expect(vars["--pg-radius-viewport-slot"]).toBe("12px");
  });

  it("emits tier variables for every tier with no backdrop blur on Minimal", () => {
    for (const tier of EFFECTS_TIERS) {
      const vars = tierCssVariables(tier);
      for (const material of GLASS_MATERIALS) {
        expect(vars[`--pg-glass-bg-${material}`]).toMatch(/^(rgb\(|#)/);
        const filter = vars[`--pg-glass-filter-${material}`];
        if (tier === "minimal") expect(filter).toBe("none");
        else expect(filter).toMatch(/^blur\(\d+px\) saturate\(\d+%\)$/);
      }
    }
  });

  it("renders a deterministic sheet with :root, tier blocks and type roles", () => {
    const sheet = fluxGlassStyleSheet();
    expect(sheet).toBe(fluxGlassStyleSheet());
    expect(sheet).toContain(":root {\n  color-scheme: dark;");
    for (const tier of EFFECTS_TIERS) expect(sheet).toContain(`[data-effects-tier="${tier}"] {`);
    expect(sheet).toContain("prefers-reduced-transparency");
    expect(sheet).toContain(".pg-type-display {");
    expect(sheet).toContain(".pg-type-data {");
  });
});

const componentCss = import.meta.glob<string>(["../components/**/*.css", "../styles/**/*.css"], { query: "?raw", import: "default", eager: true });
const componentSources = import.meta.glob<string>("../components/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
});

describe("component styles consume tokens only", () => {
  it("found the stylesheets", () => {
    expect(Object.keys(componentCss).length).toBeGreaterThanOrEqual(6);
  });

  it("every var(--pg-*) used in CSS is defined by the token sheet or locally", () => {
    const defined = new Set<string>([
      ...Object.keys(cssVariables()),
      ...Object.keys(tierCssVariables("full")),
    ]);
    for (const [file, css] of Object.entries(componentCss)) {
      for (const match of css.matchAll(/--pg-[a-z0-9-]+(?=\s*:)/g)) defined.add(match[0]);
      void file;
    }
    const missing: string[] = [];
    for (const [file, css] of Object.entries(componentCss)) {
      for (const match of css.matchAll(/var\((--pg-[a-z0-9-]+)/g)) {
        const name = match[1];
        if (name !== undefined && !defined.has(name)) missing.push(`${file}: ${name}`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("no raw hex colors in component CSS or component code", () => {
    const offenders: string[] = [];
    const hex = /#[0-9a-fA-F]{3,8}\b/g;
    for (const [file, css] of Object.entries(componentCss)) {
      const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
      for (const match of withoutComments.matchAll(hex)) offenders.push(`${file}: ${match[0]}`);
    }
    for (const [file, src] of Object.entries(componentSources)) {
      if (file.endsWith(".test.tsx") || file.endsWith(".test.ts")) continue;
      for (const match of src.matchAll(/["'`]#[0-9a-fA-F]{3,8}["'`]/g)) offenders.push(`${file}: ${match[0]}`);
      if (/tokens\/(palette|signal)\.js/.test(src)) offenders.push(`${file}: imports raw palette`);
    }
    expect(offenders).toEqual([]);
  });
});

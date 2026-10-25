import { describe, expect, it } from "vitest";
import {
  DEFAULT_EFFECTS_TIER,
  EFFECTS_TIERS,
  effectsTiers,
  GLASS_MATERIALS,
  isEffectsTier,
  resolveTransitionStyle,
  selectEffectsTier,
  type EffectsTier,
} from "./effects.js";

/** The governor's low-end reference machine (8 GB / 4 cores → Reduced). */
const LOW_END = { memoryGb: 8, cores: 4, recommended: "reduced" as EffectsTier };

describe("selectEffectsTier", () => {
  it("user override always wins", () => {
    for (const override of EFFECTS_TIERS) {
      for (const recommended of [...EFFECTS_TIERS, null]) {
        for (const lowPower of [true, false]) {
          const s = selectEffectsTier({
            recommended,
            userOverride: override,
            prefersReducedMotion: false,
            lowPower,
          });
          expect(s.tier).toBe(override);
          expect(s.source).toBe("user");
          expect(s.lowPowerApplied).toBe(false);
        }
      }
    }
  });

  it("follows the governor recommendation when there is no override", () => {
    for (const recommended of EFFECTS_TIERS) {
      for (const userOverride of [null, undefined, "auto"] as const) {
        const s = selectEffectsTier({ recommended, userOverride, prefersReducedMotion: false });
        expect(s.tier).toBe(recommended);
        expect(s.source).toBe("governor");
      }
    }
  });

  it("falls back to the default tier before the governor answers", () => {
    const s = selectEffectsTier({ recommended: null, prefersReducedMotion: false });
    expect(s.tier).toBe(DEFAULT_EFFECTS_TIER);
    expect(s.source).toBe("default");
  });

  it("Low Power caps Full to Reduced and never touches lower tiers", () => {
    expect(selectEffectsTier({ recommended: "full", prefersReducedMotion: false, lowPower: true })).toMatchObject({
      tier: "reduced",
      lowPowerApplied: true,
    });
    expect(selectEffectsTier({ recommended: "reduced", prefersReducedMotion: false, lowPower: true }).tier).toBe(
      "reduced",
    );
    expect(selectEffectsTier({ recommended: "minimal", prefersReducedMotion: false, lowPower: true }).tier).toBe(
      "minimal",
    );
  });

  it("reduced motion is orthogonal: it never changes the tier and is passed through untouched", () => {
    for (const recommended of [...EFFECTS_TIERS, null]) {
      for (const userOverride of [...EFFECTS_TIERS, "auto", null] as const) {
        const off = selectEffectsTier({ recommended, userOverride, prefersReducedMotion: false });
        const on = selectEffectsTier({ recommended, userOverride, prefersReducedMotion: true });
        expect(on.tier).toBe(off.tier);
        expect(on.source).toBe(off.source);
        expect(on.reducedMotion).toBe(true);
        expect(off.reducedMotion).toBe(false);
      }
    }
  });

  it("low-end fixture (8 GB / 4 cores) lands on Reduced", () => {
    const s = selectEffectsTier({ recommended: LOW_END.recommended, prefersReducedMotion: false });
    expect(s.tier).toBe("reduced");
  });
});

describe("tier parameters", () => {
  it("richness is monotonic Full ≥ Reduced ≥ Minimal", () => {
    const [full, reduced, minimal] = EFFECTS_TIERS.map((t) => effectsTiers[t]);
    if (!full || !reduced || !minimal) throw new Error("missing tier");
    for (const m of GLASS_MATERIALS) {
      expect(full.glass[m].blurPx).toBeGreaterThanOrEqual(reduced.glass[m].blurPx);
      expect(reduced.glass[m].blurPx).toBeGreaterThanOrEqual(minimal.glass[m].blurPx);
      // Weaker blur needs a stronger scrim.
      expect(full.glass[m].alpha).toBeLessThanOrEqual(reduced.glass[m].alpha);
      expect(reduced.glass[m].alpha).toBeLessThanOrEqual(minimal.glass[m].alpha);
    }
    expect(full.glowIntensity).toBeGreaterThan(reduced.glowIntensity);
    expect(reduced.glowIntensity).toBeGreaterThan(minimal.glowIntensity);
    expect(full.maxBackdropSurfaces).toBeGreaterThan(reduced.maxBackdropSurfaces);
  });

  it("low-end Reduced stays premium but cheap: limited blur, calm ambient", () => {
    const reduced = effectsTiers[LOW_END.recommended];
    for (const m of GLASS_MATERIALS) expect(reduced.glass[m].blurPx).toBeLessThanOrEqual(16);
    expect(reduced.ambient.amplitude).toBeLessThanOrEqual(0.5);
    expect(reduced.ambient.periodMs).toBeGreaterThanOrEqual(effectsTiers.full.ambient.periodMs);
    expect(reduced.maxBackdropSurfaces).toBeLessThanOrEqual(3);
    // Still Pegoles: glass, specular edge and some glow remain.
    expect(reduced.glass.regular.blurPx).toBeGreaterThan(0);
    expect(reduced.specular).toBeGreaterThan(0.5);
    expect(reduced.glowIntensity).toBeGreaterThan(0.5);
  });

  it("Minimal has no backdrop blur, no ambient motion — but keeps strokes and glow", () => {
    const minimal = effectsTiers.minimal;
    for (const m of GLASS_MATERIALS) expect(minimal.glass[m].blurPx).toBe(0);
    expect(minimal.ambient.enabled).toBe(false);
    expect(minimal.maxBackdropSurfaces).toBe(0);
    expect(minimal.transition).toBe("simple");
    expect(minimal.specular).toBeGreaterThan(0);
    expect(minimal.glowIntensity).toBeGreaterThan(0);
  });

  it("ambient periods stay inside the 4–8 s ambient window", () => {
    for (const tier of EFFECTS_TIERS) {
      const { periodMs } = effectsTiers[tier].ambient;
      expect(periodMs).toBeGreaterThanOrEqual(4000);
      expect(periodMs).toBeLessThanOrEqual(8000);
    }
  });
});

describe("transition style", () => {
  it("morphs only when the tier allows it and motion is not reduced", () => {
    expect(resolveTransitionStyle("full", false)).toBe("morph");
    expect(resolveTransitionStyle("reduced", false)).toBe("morph");
    expect(resolveTransitionStyle("minimal", false)).toBe("crossfade");
    for (const tier of EFFECTS_TIERS) expect(resolveTransitionStyle(tier, true)).toBe("crossfade");
  });

  it("validates tier strings", () => {
    expect(isEffectsTier("full")).toBe(true);
    expect(isEffectsTier("ultra")).toBe(false);
    expect(isEffectsTier(undefined)).toBe(false);
  });
});

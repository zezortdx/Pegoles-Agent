import { describe, expect, it } from "vitest";
import { EFFECTS_TIERS } from "../tokens/effects.js";
import { PRESENCE_STATES } from "./presenceMachine.js";
import { presenceVisual } from "./presenceVisual.js";

describe("presenceVisual — tiers choose the right layers", () => {
  it("Full: far glow, traveling light while thinking, breathing when idle", () => {
    const thinking = presenceVisual({ state: "thinking", tier: "full", reducedMotion: false, size: 96 });
    expect(thinking.layers.farGlow).toBe(true);
    expect(thinking.layers.orbit).toBe(true);
    expect(thinking.layers.arc).toBe(false);
    expect(thinking.animations.map((a) => a.keyframes)).toContain("pgm-orbit");
    const idle = presenceVisual({ state: "idle", tier: "full", reducedMotion: false, size: 96 });
    expect(idle.animations.find((a) => a.keyframes === "pgm-breathe")?.gate).toBe("ambient");
    expect(idle.eyeMotion).toBe(true);
  });

  it("Reduced: no far halo, no traveling light (static arc that shimmers), slower breath", () => {
    const thinking = presenceVisual({ state: "thinking", tier: "reduced", reducedMotion: false, size: 96 });
    expect(thinking.layers.farGlow).toBe(false);
    expect(thinking.layers.orbit).toBe(false);
    expect(thinking.layers.arc).toBe(true);
    expect(thinking.animations.map((a) => a.keyframes)).toEqual(["pgm-shimmer"]);
    const full = presenceVisual({ state: "idle", tier: "full", reducedMotion: false });
    const reduced = presenceVisual({ state: "idle", tier: "reduced", reducedMotion: false });
    const period = (v: typeof full) => v.animations.find((a) => a.target === "glow")?.durationMs ?? 0;
    expect(period(reduced)).toBeGreaterThan(period(full));
    expect(reduced.breatheMin).toBeGreaterThan(full.breatheMin);
    expect(reduced.levels.glow).toBeLessThan(full.levels.glow);
  });

  it("Minimal: a static luminous mark — no loops in any state", () => {
    for (const state of PRESENCE_STATES) {
      const v = presenceVisual({ state, tier: "minimal", reducedMotion: false });
      expect(v.animations.filter((a) => a.iterations === "infinite")).toEqual([]);
      expect(v.layers.orbit).toBe(false);
      expect(v.eyeMotion).toBe(false);
      expect(v.layers.glow).toBe(true);
    }
  });

  it("compact sizes drop detail they cannot show", () => {
    const v = presenceVisual({ state: "idle", tier: "full", reducedMotion: false, size: 20 });
    expect(v.compact).toBe(true);
    expect(v.layers.farGlow).toBe(false);
    expect(v.layers.eyeGlow).toBe(false);
  });
});

describe("presenceVisual — reduced motion", () => {
  it("has no transform animations and no loops, in every state and tier", () => {
    for (const tier of EFFECTS_TIERS) {
      for (const state of PRESENCE_STATES) {
        const v = presenceVisual({ state, tier, reducedMotion: true });
        expect(v.animations.filter((a) => a.property === "transform")).toEqual([]);
        expect(v.animations.filter((a) => a.iterations === "infinite")).toEqual([]);
        expect(v.eyeMotion).toBe(false);
        expect(v.glowShift).toBe(false);
        expect(v.layers.orbit).toBe(false);
      }
    }
  });

  it("keeps states readable through light levels", () => {
    const levels = (state: (typeof PRESENCE_STATES)[number]) =>
      presenceVisual({ state, tier: "full", reducedMotion: true }).levels;
    expect(levels("acting").charge).toBeGreaterThan(levels("thinking").charge);
    expect(levels("thinking").arc).toBeGreaterThan(0);
    expect(levels("offline").glow).toBeLessThan(levels("error").glow);
    expect(levels("error").glow).toBeLessThan(levels("idle").glow);
    expect(levels("waitingForUser").attention).toBeGreaterThan(levels("listening").attention);
    expect(presenceVisual({ state: "error", tier: "full", reducedMotion: true }).layers.badge).toBe(true);
  });
});

describe("presenceVisual — invariants", () => {
  it("only animates opacity or transform, and every loop is gated", () => {
    for (const tier of EFFECTS_TIERS) {
      for (const state of PRESENCE_STATES) {
        for (const reducedMotion of [false, true]) {
          const v = presenceVisual({ state, tier, reducedMotion });
          for (const a of v.animations) {
            expect(["opacity", "transform"]).toContain(a.property);
            if (a.iterations === "infinite") expect(["ambient", "work"]).toContain(a.gate);
          }
        }
      }
    }
  });

  it("error never paints the logo red: it lowers energy and adds a badge", () => {
    const v = presenceVisual({ state: "error", tier: "full", reducedMotion: false });
    expect(v.layers.badge).toBe(true);
    expect(v.levels.dim).toBeGreaterThan(0);
    expect(v.levels.glow).toBeLessThan(presenceVisual({ state: "idle", tier: "full", reducedMotion: false }).levels.glow);
  });

  it("offline is almost unlit", () => {
    const v = presenceVisual({ state: "offline", tier: "full", reducedMotion: false });
    expect(v.levels.glow).toBeLessThanOrEqual(0.1);
    expect(v.animations).toEqual([]);
  });
});

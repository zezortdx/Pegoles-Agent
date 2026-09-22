import { describe, expect, it } from "vitest";
import { MAX_EYE_SHIFT_PX, attentionAngle, eyeOffset, lookToward } from "./geometry.js";

const len = (v: { x: number; y: number }) => Math.hypot(v.x, v.y);

describe("eyeOffset", () => {
  it("never exceeds 3 px, at any size or vector magnitude", () => {
    for (const size of [0, 20, 32, 64, 100, 160, 400, 10_000]) {
      for (const v of [
        { x: 1, y: 0 },
        { x: -1, y: 1 },
        { x: 50, y: -80 },
        { x: 1e9, y: 1e9 },
        { x: 0.2, y: 0.1 },
      ]) {
        expect(len(eyeOffset(v, size))).toBeLessThanOrEqual(MAX_EYE_SHIFT_PX + 1e-9);
      }
    }
  });

  it("reaches 3 px on large marks and stays sub-pixel-ish on the nav rail", () => {
    expect(len(eyeOffset({ x: 1, y: 0 }, 160))).toBeCloseTo(3, 5);
    expect(len(eyeOffset({ x: 0, y: 1 }, 20))).toBeCloseTo(0.6, 5);
    expect(len(eyeOffset({ x: 0, y: 1 }, 64))).toBeCloseTo(1.92, 2);
  });

  it("is proportional below unit length and zero without a target", () => {
    expect(len(eyeOffset({ x: 0.5, y: 0 }, 160))).toBeCloseTo(1.5, 5);
    expect(eyeOffset(null, 160)).toEqual({ x: 0, y: 0 });
    expect(eyeOffset({ x: 0, y: 0 }, 160)).toEqual({ x: 0, y: 0 });
  });

  it("sanitizes non-finite input", () => {
    expect(eyeOffset({ x: Number.NaN, y: Number.POSITIVE_INFINITY }, 160)).toEqual({ x: 0, y: 0 });
    expect(eyeOffset({ x: 1, y: 0 }, Number.NaN)).toEqual({ x: 0, y: 0 });
  });
});

describe("lookToward / attentionAngle", () => {
  it("points toward content and saturates at range", () => {
    const v = lookToward({ x: 0, y: 0 }, { x: 1000, y: 0 }, 240);
    expect(v).toEqual({ x: 1, y: 0 });
    const near = lookToward({ x: 0, y: 0 }, { x: 0, y: 120 }, 240);
    expect(near.y).toBeCloseTo(0.5);
  });

  it("defaults to down (the command surface) and accepts degrees or vectors", () => {
    expect(attentionAngle(null)).toBeCloseTo(Math.PI / 2);
    expect(attentionAngle(0)).toBe(0);
    expect(attentionAngle({ x: -1, y: 0 })).toBeCloseTo(Math.PI);
  });
});

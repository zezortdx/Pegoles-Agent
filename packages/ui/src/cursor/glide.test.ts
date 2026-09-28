import { describe, expect, it } from "vitest";
import { DRAG_MAX_MS, DRAG_MIN_MS, GLIDE, dragDuration, glideDuration, sampleGlide, startGlide } from "./glide.js";

describe("glideDuration", () => {
  it("lands at once for tiny moves and under reduced motion", () => {
    expect(glideDuration(1, false)).toBe(0);
    expect(glideDuration(400, true)).toBe(0);
    expect(glideDuration(Number.NaN, false)).toBe(0);
  });

  it("is nearly immediate close by, a little longer far away, never slow", () => {
    const near = glideDuration(6, false);
    const mid = glideDuration(120, false);
    const far = glideDuration(900, false);
    expect(near).toBeLessThan(100);
    expect(mid).toBeGreaterThan(near);
    expect(far).toBeGreaterThan(mid);
    expect(far).toBeLessThanOrEqual(GLIDE.maxMs);
    expect(mid).toBeGreaterThan(150);
    expect(mid).toBeLessThan(300);
  });

  it("keeps a drag at the real drag's pace, within bounds", () => {
    expect(dragDuration(500, false)).toBe(500);
    expect(dragDuration(10, false)).toBe(DRAG_MIN_MS);
    expect(dragDuration(60_000, false)).toBe(DRAG_MAX_MS);
    expect(dragDuration(500, true)).toBe(0);
  });
});

describe("glide", () => {
  const from = { x: 0.1, y: 0.1 };
  const to = { x: 0.9, y: 0.5 };

  it("starts where it is, ends exactly on the target, and eases in and out", () => {
    const g = startGlide(from, to, { x: 0, y: 0 }, 0, 300);
    expect(sampleGlide(g, 0).position).toEqual(from);
    expect(sampleGlide(g, 300)).toMatchObject({ position: to, done: true });
    const early = sampleGlide(g, 30).velocity.x;
    const middle = sampleGlide(g, 150).velocity.x;
    const late = sampleGlide(g, 285).velocity.x;
    expect(middle).toBeGreaterThan(early);
    expect(middle).toBeGreaterThan(late);
    // Monotonic progress: no overshoot, no wobble.
    let last = -Infinity;
    for (let t = 0; t <= 300; t += 10) {
      const x = sampleGlide(g, t).position.x;
      expect(x).toBeGreaterThanOrEqual(last);
      expect(x).toBeLessThanOrEqual(to.x + 1e-12);
      last = x;
    }
  });

  it("keeps velocity when retargeted in a similar direction (no stall, no jump)", () => {
    const g = startGlide(from, to, { x: 0, y: 0 }, 0, 300);
    const at = sampleGlide(g, 120);
    const next = startGlide(at.position, { x: 0.95, y: 0.6 }, at.velocity, 120, 280);
    const first = sampleGlide(next, 120);
    expect(first.position.x).toBeCloseTo(at.position.x, 12);
    expect(first.position.y).toBeCloseTo(at.position.y, 12);
    expect(first.velocity.x).toBeGreaterThan(0);
  });

  it("drops velocity pointing away from a reversed target, so it never swings back", () => {
    const g = startGlide(from, to, { x: 0, y: 0 }, 0, 300);
    const at = sampleGlide(g, 150);
    const back = startGlide(at.position, from, at.velocity, 150, 300);
    expect(back.velocity).toEqual({ x: 0, y: 0 });
    for (let t = 150; t <= 450; t += 15) expect(sampleGlide(back, t).position.x).toBeLessThanOrEqual(at.position.x + 1e-12);
  });

  it("caps carried velocity so a short hop is not flung past its target", () => {
    const hop = startGlide({ x: 0.5, y: 0.5 }, { x: 0.52, y: 0.5 }, { x: 1, y: 0 }, 0, 100);
    expect(hop.velocity.x).toBeCloseTo((GLIDE.carryLimit * 0.02) / 100, 12);
  });

  it("a zero-length or zero-time glide is already done", () => {
    expect(sampleGlide(startGlide(to, to, { x: 1, y: 1 }, 0, 200), 0)).toMatchObject({ position: to, done: true });
    expect(sampleGlide(startGlide(from, to, { x: 0, y: 0 }, 0, 0), 0)).toMatchObject({ position: to, done: true });
  });
});

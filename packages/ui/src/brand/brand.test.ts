import { describe, expect, it } from "vitest";
import { MARK_ARTBOARD, MARK_GEOMETRY, MARK_RING_MASK, MARK_RING_PATH, markGlyphSvg } from "./index.js";

const NUM = /-?\d*\.?\d+/g;

function pathBounds(d: string) {
  const nums = (d.match(NUM) ?? []).map(Number);
  const xs = nums.filter((_, i) => i % 2 === 0);
  const ys = nums.filter((_, i) => i % 2 === 1);
  return { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) };
}

describe("brand geometry (generated from the traced PNG)", () => {
  it("uses only absolute M/L/C/Z commands inside the artboard", () => {
    for (const d of Object.values(MARK_GEOMETRY.paths)) {
      expect(d).toMatch(/^M[\d.\sLCZ-]+Z$/);
      const b = pathBounds(d);
      expect(b.minX).toBeGreaterThanOrEqual(0);
      expect(b.minY).toBeGreaterThanOrEqual(0);
      expect(b.maxX).toBeLessThanOrEqual(MARK_ARTBOARD);
      expect(b.maxY).toBeLessThanOrEqual(MARK_ARTBOARD);
    }
  });

  it("keeps the mark's proportions: eyes inside the opening, opening inside the ring", () => {
    const { bounds, opening, eyes } = MARK_GEOMETRY;
    expect(opening.x).toBeGreaterThan(bounds.x);
    expect(opening.x + opening.width).toBeLessThan(bounds.x + bounds.width);
    for (const eye of eyes) {
      expect(eye.cx - eye.width / 2).toBeGreaterThan(opening.x);
      expect(eye.cx + eye.width / 2).toBeLessThan(opening.x + opening.width);
      expect(eye.height / eye.width).toBeGreaterThan(2);
    }
    // Mark is centred in the artboard (outer bbox centre = artboard centre).
    expect(bounds.x + bounds.width / 2).toBeCloseTo(MARK_ARTBOARD / 2, 0);
    expect(bounds.y + bounds.height / 2).toBeCloseTo(MARK_ARTBOARD / 2, 0);
  });

  it("builds flat glyph SVGs and a CSS ring mask", () => {
    expect(MARK_RING_PATH.startsWith(MARK_GEOMETRY.paths.outer)).toBe(true);
    const svg = markGlyphSvg({ color: "#fff", eyes: false });
    expect(svg).toContain('fill-rule="evenodd"');
    expect(svg).not.toContain(MARK_GEOMETRY.paths.eyeLeft);
    expect(MARK_RING_MASK.startsWith('url("data:image/svg+xml,')).toBe(true);
  });

  it("only uses sampled/brand colors (no invented hues)", () => {
    const { colors } = MARK_GEOMETRY;
    const all = [
      ...colors.ringBody,
      ...colors.ringLight,
      ...colors.ringRim,
      ...colors.ringInnerEdge,
      ...colors.eye,
    ].map((s) => s.color);
    for (const c of all) {
      const r = parseInt(c.slice(1, 3), 16);
      const b = parseInt(c.slice(5, 7), 16);
      expect(b).toBeGreaterThanOrEqual(r); // blue/cyan/ice family only
    }
  });
});

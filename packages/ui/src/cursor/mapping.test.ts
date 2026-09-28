import { describe, expect, it } from "vitest";
import { clampUnit, containRect, frameRect, snapToDevice, toOverlay } from "./mapping.js";

describe("containRect (object-fit: contain)", () => {
  it("fills the box when aspect ratios match", () => {
    expect(containRect({ width: 720, height: 450 }, { width: 1440, height: 900 })).toEqual({ x: 0, y: 0, width: 720, height: 450 });
  });

  it("letterboxes a wide frame in a tall box", () => {
    const r = containRect({ width: 800, height: 800 }, { width: 1600, height: 900 });
    expect(r.width).toBe(800);
    expect(r.height).toBe(450);
    expect(r.x).toBe(0);
    expect(r.y).toBe(175);
  });

  it("pillarboxes a tall frame in a wide box", () => {
    const r = containRect({ width: 1000, height: 500 }, { width: 1024, height: 768 });
    expect(r.height).toBe(500);
    expect(r.width).toBeCloseTo(666.667, 2);
    expect(r.x).toBeCloseTo(166.667, 2);
    expect(r.y).toBe(0);
  });

  it("falls back to the whole box without a usable frame size", () => {
    expect(containRect({ width: 300, height: 200 }, null)).toEqual({ x: 0, y: 0, width: 300, height: 200 });
    expect(containRect({ width: 300, height: 200 }, { width: 0, height: 900 })).toEqual({ x: 0, y: 0, width: 300, height: 200 });
    expect(containRect({ width: Number.NaN, height: 200 }, { width: 10, height: 10 })).toEqual({ x: 0, y: 0, width: 0, height: 200 });
  });
});

describe("frameRect (object-fit: cover)", () => {
  it("crops instead of letterboxing: the frame overflows the box, centred", () => {
    const r = frameRect({ width: 320, height: 200 }, { width: 1024, height: 768 }, "cover");
    expect(r.width).toBe(320);
    expect(r.height).toBe(240);
    expect(r.y).toBe(-20);
    expect(toOverlay({ x: 0.5, y: 0.5 }, r)).toEqual({ x: 160, y: 100 });
  });
});

describe("toOverlay", () => {
  const frame = { x: 0, y: 175, width: 800, height: 450 };

  it("maps normalized guest points into the drawn frame, not the box", () => {
    expect(toOverlay({ x: 0, y: 0 }, frame)).toEqual({ x: 0, y: 175 });
    expect(toOverlay({ x: 1, y: 1 }, frame)).toEqual({ x: 800, y: 625 });
    expect(toOverlay({ x: 0.5, y: 0.5 }, frame)).toEqual({ x: 400, y: 400 });
  });

  it("is independent of guest resolution and DPI: the same normalized point lands in the same place", () => {
    // A 2880x1800 Retina-class guest and a 1440x900 one both fit the same box identically.
    const box = { width: 960, height: 600 };
    const a = containRect(box, { width: 2880, height: 1800 });
    const b = containRect(box, { width: 1440, height: 900 });
    expect(toOverlay({ x: 0.25, y: 0.75 }, a)).toEqual(toOverlay({ x: 0.25, y: 0.75 }, b));
  });

  it("clamps stray coordinates to the frame", () => {
    expect(toOverlay({ x: -3, y: 9 }, frame)).toEqual({ x: 0, y: 625 });
    expect(toOverlay({ x: Number.NaN, y: Number.POSITIVE_INFINITY }, frame)).toEqual({ x: 0, y: 175 });
    expect(clampUnit(0.4)).toBe(0.4);
  });
});

describe("snapToDevice", () => {
  it("rounds to the device pixel grid", () => {
    expect(snapToDevice(10.3, 2)).toBe(10.5);
    expect(snapToDevice(10.3, 1)).toBe(10);
    expect(snapToDevice(10.3, 0)).toBe(10);
  });
});

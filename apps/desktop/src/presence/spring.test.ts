import { describe, expect, it } from "vitest";
import { isSettled, stepCritical, type Spring } from "./spring";

describe("critically damped spring", () => {
  it("is frame-rate independent: one 100 ms step equals six 16.7 ms steps", () => {
    const coarse: Spring = { position: 0, velocity: 3 };
    const fine: Spring = { position: 0, velocity: 3 };
    stepCritical(coarse, 1, 0.42, 0.1);
    for (let i = 0; i < 6; i += 1) stepCritical(fine, 1, 0.42, 0.1 / 6);
    expect(coarse.position).toBeCloseTo(fine.position, 9);
    expect(coarse.velocity).toBeCloseTo(fine.velocity, 9);
  });

  it("never overshoots from rest and settles", () => {
    const spring: Spring = { position: 0, velocity: 0 };
    let peak = 0;
    for (let i = 0; i < 120; i += 1) {
      stepCritical(spring, 1, 0.2, 1 / 60);
      peak = Math.max(peak, spring.position);
    }
    expect(peak).toBeLessThanOrEqual(1);
    expect(isSettled(spring, 1)).toBe(true);
  });

  it("ignores non-positive time steps", () => {
    const spring: Spring = { position: 0.5, velocity: 1 };
    stepCritical(spring, 1, 0.2, 0);
    expect(spring).toEqual({ position: 0.5, velocity: 1 });
  });
});

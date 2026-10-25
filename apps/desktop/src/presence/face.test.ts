import { describe, expect, it } from "vitest";
import { liveliness } from "./engine/face";

describe("spontaneous avatar blinks", () => {
  it.each([0, 910, 1_800, 43_200, 86_400])("still closes and reopens the eyes after %i seconds", (start) => {
    const lids = Array.from({ length: 800 }, (_, index) => liveliness(start + index / 100).lid);
    expect(Math.min(...lids)).toBeLessThan(0.2);
    expect(Math.max(...lids)).toBe(1);
    expect(lids.every((lid) => Number.isFinite(lid) && lid >= 0 && lid <= 1)).toBe(true);
  });

  it("keeps the original first blink timing", () => {
    expect(liveliness(1.3).lid).toBe(1);
    expect(liveliness(1.481).lid).toBeCloseTo(0);
    expect(liveliness(1.6).lid).toBe(1);
  });

  it("keeps blinking deterministic and disabled when requested in long sessions", () => {
    for (let index = 0; index < 800; index++) {
      const time = 43_200 + index / 100;
      const lid = liveliness(time).lid;
      liveliness(time + 10_000);
      expect(liveliness(time).lid).toBe(lid);
      expect(liveliness(time, { blink: false }).lid).toBe(1);
    }
  });
});

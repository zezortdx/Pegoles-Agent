import { describe, expect, it } from "vitest";
import { BotEngine } from "./engine/engine";
import { STATES } from "./engine/states";

describe("vendored bloub engine", () => {
  it.each(STATES)("renders finite and repeatable SVG geometry for $id", ({ id }) => {
    const engine = new BotEngine(100, id);
    const first = engine.sample(0.9);
    for (const time of [0, 0.1, 1.2, 2.5, 10, 500]) {
      const frame = engine.sample(time);
      expect(JSON.stringify(frame)).not.toMatch(/NaN|Infinity/);
      expect(frame.bodyPath).toMatch(/^M.+Z$/);
      for (const eye of frame.eyes) expect(eye.matrix).toMatch(/^matrix\(/);
    }
    expect(engine.sample(0.9)).toEqual(first);
  });

  it("ignores a non-finite pointer target without poisoning later frames", () => {
    const engine = new BotEngine();
    const before = engine.sample(1);
    engine.setLook({ yaw: NaN, pitch: 0, mix: 1, spin: 0, wander: 0 }, 1);
    expect(engine.sample(1)).toEqual(before);
  });
});

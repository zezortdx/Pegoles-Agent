import { describe, expect, it } from "vitest";
import { columns, SIDEBAR_WIDTH, stepBack, WORK_MIN } from "./layout";

describe("computer columns", () => {
  it("is closed without a level", () => {
    expect(columns(null, 1440, true)).toEqual({ sidebar: SIDEBAR_WIDTH, computer: 0 });
    expect(columns(null, 1440, false)).toEqual({ sidebar: 0, computer: 0 });
  });

  it("shares the window at Side and never squeezes the task below its minimum", () => {
    const wide = columns("side", 1440, true);
    expect(wide.computer).toBe(Math.round((1440 - SIDEBAR_WIDTH) * 0.42));
    const narrow = columns("side", 960, true);
    expect(960 - narrow.sidebar - narrow.computer).toBeGreaterThanOrEqual(WORK_MIN);
  });

  it("gives Focus most of the room and keeps a readable task column", () => {
    const focus = columns("focus", 1440, true);
    const work = 1440 - focus.sidebar - focus.computer;
    expect(work).toBeGreaterThanOrEqual(340);
    expect(work).toBeLessThanOrEqual(420);
  });

  it("hands Full the whole window", () => {
    expect(columns("full", 1440, true)).toEqual({ sidebar: 0, computer: 1440 });
  });

  it("steps back one level at a time", () => {
    expect(stepBack("full")).toBe("focus");
    expect(stepBack("focus")).toBe("side");
    expect(stepBack("side")).toBeNull();
  });
});

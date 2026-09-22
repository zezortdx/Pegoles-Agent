import { describe, expect, it } from "vitest";
import { CURSOR_STATES, canTransition, transitionCursor, type CursorEvent, type CursorState } from "./cursorMachine.js";

describe("AgentCursor state machine", () => {
  it("starts hidden and only `show` leaves hidden", () => {
    const events: CursorEvent[] = ["move", "arrive", "click", "clickEnd", "dragStart", "dragEnd", "typeStart", "typeEnd", "wait", "hide"];
    for (const e of events) expect(transitionCursor("hidden", e)).toBe("hidden");
    expect(transitionCursor("hidden", "show")).toBe("idle");
  });

  it("hide works from every visible state (human takes control)", () => {
    for (const s of CURSOR_STATES) expect(transitionCursor(s, "hide")).toBe("hidden");
  });

  it("walks move → click → drag → type → wait", () => {
    let s: CursorState = "idle";
    const step = (e: CursorEvent, expected: CursorState) => {
      s = transitionCursor(s, e);
      expect(s).toBe(expected);
    };
    step("move", "moving");
    step("move", "moving");
    step("arrive", "idle");
    step("click", "clicking");
    step("clickEnd", "idle");
    step("dragStart", "dragging");
    step("dragEnd", "idle");
    step("typeStart", "typing");
    step("typeEnd", "idle");
    step("wait", "waiting");
    step("move", "moving");
  });

  it("rejects invalid transitions", () => {
    expect(canTransition("dragging", "click")).toBe(false);
    expect(transitionCursor("dragging", "click")).toBe("dragging");
    expect(transitionCursor("dragging", "typeStart")).toBe("dragging");
    expect(transitionCursor("idle", "arrive")).toBe("idle");
    expect(transitionCursor("idle", "clickEnd")).toBe("idle");
    expect(transitionCursor("typing", "dragStart")).toBe("typing");
    expect(transitionCursor("waiting", "wait")).toBe("waiting");
    expect(canTransition("idle", "show")).toBe(false);
  });
});

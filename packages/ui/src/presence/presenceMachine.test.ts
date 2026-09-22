import { describe, expect, it, vi } from "vitest";
import {
  PRESENCE_STATES,
  SUCCESS_HOLD_MS,
  createPresenceMachine,
  transitionPresence,
  type PresenceEvent,
  type PresenceState,
} from "./presenceMachine.js";

const ev = (type: PresenceEvent["type"]): PresenceEvent => ({ type }) as PresenceEvent;

describe("transitionPresence", () => {
  it("walks a normal task lifecycle", () => {
    let s: PresenceState = "idle";
    s = transitionPresence(s, ev("inputFocused"));
    expect(s).toBe("listening");
    s = transitionPresence(s, ev("taskStarted"));
    expect(s).toBe("thinking");
    s = transitionPresence(s, ev("acting"));
    expect(s).toBe("acting");
    s = transitionPresence(s, ev("needsUser"));
    expect(s).toBe("waitingForUser");
    s = transitionPresence(s, ev("acting"));
    expect(s).toBe("acting");
    s = transitionPresence(s, ev("succeeded"));
    expect(s).toBe("success");
    s = transitionPresence(s, ev("elapsed"));
    expect(s).toBe("idle");
  });

  it("ignores invalid events (returns the same state)", () => {
    expect(transitionPresence("idle", ev("succeeded"))).toBe("idle");
    expect(transitionPresence("idle", ev("elapsed"))).toBe("idle");
    expect(transitionPresence("thinking", ev("inputFocused"))).toBe("thinking");
    expect(transitionPresence("acting", ev("online"))).toBe("acting");
    expect(transitionPresence("success", ev("acting"))).toBe("success");
    expect(transitionPresence("error", ev("failed"))).toBe("error");
  });

  it("offline swallows everything except online", () => {
    for (const type of ["taskStarted", "thinking", "acting", "needsUser", "succeeded", "failed", "inputFocused", "idleTimeout"] as const) {
      expect(transitionPresence("offline", ev(type))).toBe("offline");
    }
    expect(transitionPresence("offline", ev("online"))).toBe("idle");
  });

  it("goes offline from every online state", () => {
    for (const s of PRESENCE_STATES) {
      expect(transitionPresence(s, ev("offline"))).toBe("offline");
    }
  });

  it("fails into error from any online state, and recovers on a new task or focus", () => {
    for (const s of PRESENCE_STATES.filter((x) => x !== "offline")) {
      expect(transitionPresence(s, ev("failed"))).toBe("error");
    }
    expect(transitionPresence("error", ev("taskStarted"))).toBe("thinking");
    expect(transitionPresence("error", ev("inputFocused"))).toBe("listening");
    expect(transitionPresence("error", ev("idleTimeout"))).toBe("idle");
  });

  it("cancellation of work settles to idle", () => {
    for (const s of ["thinking", "acting", "waitingForUser"] as const) {
      expect(transitionPresence(s, ev("cancelled"))).toBe("idle");
    }
  });
});

describe("createPresenceMachine", () => {
  it("returns from Success to Idle after the hold (timed transition)", () => {
    vi.useFakeTimers();
    try {
      const m = createPresenceMachine("acting");
      const seen: PresenceState[] = [];
      m.subscribe((s) => seen.push(s));
      m.send(ev("succeeded"));
      expect(m.state).toBe("success");
      vi.advanceTimersByTime(SUCCESS_HOLD_MS - 1);
      expect(m.state).toBe("success");
      vi.advanceTimersByTime(1);
      expect(m.state).toBe("idle");
      expect(seen).toEqual(["success", "idle"]);
      m.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels the Success timer when the state changes first", () => {
    vi.useFakeTimers();
    try {
      const m = createPresenceMachine("thinking");
      m.send(ev("succeeded"));
      m.send(ev("taskStarted"));
      expect(m.state).toBe("thinking");
      vi.advanceTimersByTime(SUCCESS_HOLD_MS * 2);
      expect(m.state).toBe("thinking");
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not notify for ignored events", () => {
    const m = createPresenceMachine("idle");
    const listener = vi.fn();
    m.subscribe(listener);
    m.send(ev("succeeded"));
    m.send(ev("elapsed"));
    expect(listener).not.toHaveBeenCalled();
  });

  it("dispose releases the pending timer", () => {
    vi.useFakeTimers();
    try {
      const m = createPresenceMachine("acting");
      m.send(ev("succeeded"));
      m.dispose();
      vi.advanceTimersByTime(SUCCESS_HOLD_MS * 2);
      expect(m.state).toBe("success");
    } finally {
      vi.useRealTimers();
    }
  });
});

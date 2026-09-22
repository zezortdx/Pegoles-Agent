import { describe, expect, it } from "vitest";
import { createPresenceMachine, transitionPresence, type PresenceState } from "./presenceMachine.js";
import {
  EMPTY_PRESENCE_SNAPSHOT as EMPTY,
  derivePresence,
  presenceEventsForChange,
  type PresenceSnapshot,
} from "./presenceSource.js";

function snap(p: Partial<PresenceSnapshot>): PresenceSnapshot {
  return { ...EMPTY, ...p };
}

/** Feed a sequence of snapshots through the event mapping + machine. */
function run(snapshots: PresenceSnapshot[]): PresenceState[] {
  const first = snapshots[0] ?? EMPTY;
  const m = createPresenceMachine(derivePresence(first), {
    setTimeout: () => 0,
    clearTimeout: () => undefined,
  });
  const out: PresenceState[] = [m.state];
  let prev = first;
  for (const next of snapshots.slice(1)) {
    for (const e of presenceEventsForChange(prev, next)) m.send(e);
    out.push(m.state);
    prev = next;
  }
  return out;
}

describe("derivePresence (real app state → steady presence)", () => {
  it("maps Core facts", () => {
    expect(derivePresence(snap({ online: false, task: "running" }))).toBe("offline");
    expect(derivePresence(snap({}))).toBe("idle");
    expect(derivePresence(snap({ inputFocused: true }))).toBe("listening");
    expect(derivePresence(snap({ task: "pending" }))).toBe("idle");
    expect(run([snap({}), snap({ task: "pending" })])).toEqual(["idle", "idle"]);
    expect(derivePresence(snap({ task: "running", viewport: "ready" }))).toBe("thinking");
    expect(derivePresence(snap({ task: "running", viewport: "agent_active" }))).toBe("acting");
    expect(derivePresence(snap({ task: "running", viewport: "user_controlled" }))).toBe("waitingForUser");
    expect(derivePresence(snap({ task: "waiting_for_approval" }))).toBe("waitingForUser");
    expect(derivePresence(snap({ task: "failed" }))).toBe("error");
    expect(derivePresence(snap({ task: "running", viewport: "error" }))).toBe("error");
    expect(derivePresence(snap({ task: "completed" }))).toBe("idle");
  });
});

describe("presenceEventsForChange", () => {
  it("completion of a working task shows Success (machine returns to Idle on its own)", () => {
    expect(run([snap({ task: "running" }), snap({ task: "completed" })])).toEqual(["thinking", "success"]);
  });

  it("a completed task seen at startup does not fake a Success", () => {
    expect(run([snap({ task: "completed" }), snap({ task: "completed", inputFocused: false })])).toEqual(["idle", "idle"]);
  });

  it("tracks the agent acting on its computer and waiting for the user", () => {
    expect(
      run([
        snap({}),
        snap({ inputFocused: true }),
        snap({ task: "running", inputFocused: false }),
        snap({ task: "running", viewport: "agent_active" }),
        snap({ task: "waiting_for_approval", viewport: "agent_active" }),
        snap({ task: "running", viewport: "agent_active" }),
        snap({ task: "failed", viewport: "ready" }),
        snap({ task: null, viewport: "ready" }),
      ]),
    ).toEqual(["idle", "listening", "thinking", "acting", "waitingForUser", "acting", "error", "idle"]);
  });

  it("offline and back online", () => {
    expect(run([snap({ task: "running" }), snap({ online: false, task: "running" }), snap({ task: "running" })])).toEqual([
      "thinking",
      "offline",
      "thinking",
    ]);
  });

  it("cancel settles to idle; blur leaves listening", () => {
    expect(run([snap({ task: "running" }), snap({ task: "cancelled" })])).toEqual(["thinking", "idle"]);
    expect(run([snap({ inputFocused: true }), snap({ inputFocused: false })])).toEqual(["listening", "idle"]);
  });

  it("emits nothing when nothing changed", () => {
    const s = snap({ task: "running", viewport: "agent_active" });
    expect(presenceEventsForChange(s, s)).toEqual([]);
  });

  it("every emitted event is valid from the machine's state", () => {
    const seq = [
      snap({}),
      snap({ task: "pending" }),
      snap({ task: "running", viewport: "agent_active" }),
      snap({ task: "running", viewport: "user_controlled" }),
      snap({ task: "completed" }),
    ];
    let state: PresenceState = derivePresence(seq[0] as PresenceSnapshot);
    for (let i = 1; i < seq.length; i += 1) {
      for (const e of presenceEventsForChange(seq[i - 1] as PresenceSnapshot, seq[i] as PresenceSnapshot)) {
        const next = transitionPresence(state, e);
        expect(next, `${state} + ${e.type}`).not.toBe(undefined);
        state = next;
      }
    }
    expect(state).toBe("success");
  });
});

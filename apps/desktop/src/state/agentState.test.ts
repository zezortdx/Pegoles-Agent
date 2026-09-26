import { describe, expect, it } from "vitest";
import { actionSteps, globalPresence, modelConnected, taskActivity } from "./agentState";
import type { ActionRequestWire, AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";

const at = "2026-09-24T10:00:00Z";
const task: AgentTask = { id: "t1", title: "Read report", status: "running", created_at: at, updated_at: at };
const request: ActionRequestWire = { action_id: "a1", task_id: "t1", computer_id: "vm1", action: { type: "click" }, requested_at: at };
const start: AgentEvent = { type: "action_started", request, action_id: "a1", at };
const done: AgentEvent = { type: "action_completed", request, result: { action_id: "a1", success: true, outcome: "executed", message: "", duration_ms: 20 } };
const busy = { agent_busy: true, control_owner: "agent", model: "configured" } as StatusPayload;
const idle = { agent_busy: false, control_owner: "none", model: "not_configured" } as StatusPayload;

const activity = (overrides: Partial<Parameters<typeof taskActivity>[0]>) =>
  taskActivity({ task, events: [], status: busy, connected: true, ...overrides });

describe("taskActivity: real state in, presence out", () => {
  it("believes a started action only while Core says input is busy", () => {
    expect(activity({ events: [start] })).toMatchObject({ mode: "using-computer", capability: "Computer", headline: "Using its computer", detail: "Clicking" });
    expect(activity({ events: [start], status: { ...busy, agent_busy: false } }).mode).toBe("thinking");
    expect(activity({ events: [start, done] }).mode).toBe("thinking");
    expect(activity({ events: [start], connected: false }).mode).toBe("offline");
  });

  it("is honest about pending tasks when no model is connected", () => {
    const pending = activity({ task: { ...task, status: "pending" }, status: idle });
    expect(pending).toMatchObject({ mode: "blocked", headline: "Can’t start yet", offerComputer: true, live: true, start: "needs-model" });
    expect(pending.detail).toMatch(/Anthropic API key in Settings/);
  });

  it("says a pending task hasn't started once a model is connected, and why it can't start yet", () => {
    const pending = { ...task, status: "pending" as const };
    const ready = { ...busy, agent_busy: false, control_owner: "none", active_task: null } as StatusPayload;
    expect(activity({ task: pending, status: ready })).toMatchObject({ mode: "idle", headline: "Not started", live: false, start: "ready" });
    expect(activity({ task: pending, status: { ...ready, active_task: "other" } })).toMatchObject({ start: "busy", detail: "Pegoles is working on another task" });
    // Starting: the request is in flight, or Core has claimed the run but not moved the task yet.
    const starting = activity({ task: pending, status: ready, starting: true });
    expect(starting).toMatchObject({ mode: "thinking", headline: "Starting", live: true });
    expect(starting.start).toBeUndefined();
    expect(activity({ task: pending, status: { ...ready, active_task: "t1" } })).toMatchObject({ mode: "thinking", headline: "Starting" });
  });

  it("only believes Core about the model", () => {
    expect(modelConnected({ model: "configured" } as StatusPayload)).toBe(true);
    expect(modelConnected({ model: "not_configured" } as StatusPayload)).toBe(false);
    expect(modelConnected(null)).toBe(false);
  });

  it("shows Pegoles' latest note between actions, and why a finished run stopped", () => {
    const note = (kind: string, text: string, s: number): AgentEvent =>
      ({ type: "agent_message", task_id: "t1", kind, text, at: `2026-09-24T10:00:0${s}Z` }) as AgentEvent;
    const thinking = activity({ events: [note("progress", "Opening the browser", 1), note("progress", "  \nSearching for pricing pages\nthen comparing", 2)], status: { ...busy, agent_busy: false } });
    expect(thinking).toMatchObject({ mode: "thinking", headline: "Working on it", detail: "Searching for pricing pages" });
    const stopped = activity({ task: { ...task, status: "cancelled" }, events: [note("error", "Stopped by the user.", 3)] });
    expect(stopped).toMatchObject({ mode: "idle", headline: "Cancelled", reason: "Stopped by the user." });
    expect(activity({ task: { ...task, status: "failed" }, events: [note("error", "The model service refused the key.", 3)] }).reason).toBe("The model service refused the key.");
  });

  it("acknowledges a new hand-off but never hides real work behind it", () => {
    expect(activity({ task: { ...task, status: "pending" }, status: idle, acknowledging: true }).mode).toBe("acknowledging");
    expect(activity({ events: [start], acknowledging: true }).mode).toBe("using-computer");
  });

  it("waits while the person holds the computer, and needs them for approvals", () => {
    expect(activity({ events: [start], status: { ...busy, control_owner: "user" } }).mode).toBe("waiting");
    const approval: AgentEvent = { type: "approval_requested", task_id: "t1", reason: "Open example.com", at };
    expect(activity({ task: { ...task, status: "waiting_for_approval" }, events: [approval] })).toMatchObject({ mode: "needs-user", approvalReason: "Open example.com" });
  });

  it("settles on terminal states and keeps a policy stop visible", () => {
    expect(activity({ task: { ...task, status: "completed" } })).toMatchObject({ mode: "done", live: false, at });
    const denied: AgentEvent = { type: "action_denied", request, reason: "host path" };
    expect(activity({ events: [start, denied] })).toMatchObject({ mode: "blocked", headline: "Stopped by a safety rule" });
    expect(activity({ task: { ...task, status: "failed" }, events: [start, denied] })).toMatchObject({ mode: "error", detail: "Clicked" });
  });

  it("never shows typed text and counts pulses from real events", () => {
    const secret = { ...start, request: { ...request, action: { type: "type_text", text: "hunter2", sensitive: true } } };
    const result = activity({ events: [secret] });
    expect(result.detail).toBe("Typing");
    expect(JSON.stringify(result)).not.toContain("hunter2");
    expect(result.pulse).toBe(1);
  });
});

describe("actionSteps", () => {
  it("keeps one step per action and never reopens a finished one", () => {
    const echo: AgentEvent = { ...start, at: "2026-09-24T10:00:02Z" };
    expect(actionSteps([start, done, echo])).toEqual([expect.objectContaining({ id: "a1", outcome: "done", at })]);
  });
  it("shows only safe targets", () => {
    const open = { ...start, request: { ...request, action: { type: "open_url", url: "https://example.com/private?q=1" } } };
    expect(actionSteps([open])[0]).toMatchObject({ capability: "Web", detail: "example.com" });
  });
});

describe("globalPresence", () => {
  const base = { connected: true, tasks: [] as AgentTask[], status: idle, acknowledging: false, attentive: false, computerTransitioning: false };
  it("orders what matters most", () => {
    expect(globalPresence({ ...base, connected: false })).toBe("offline");
    expect(globalPresence({ ...base, acknowledging: true })).toBe("acknowledging");
    expect(globalPresence({ ...base, tasks: [{ ...task, status: "waiting_for_approval" }], status: busy })).toBe("needs-user");
    expect(globalPresence({ ...base, status: busy })).toBe("using-computer");
    expect(globalPresence({ ...base, tasks: [task] })).toBe("thinking");
    expect(globalPresence({ ...base, computerTransitioning: true })).toBe("working");
    expect(globalPresence({ ...base, attentive: true })).toBe("attentive");
    expect(globalPresence(base)).toBe("idle");
  });
});

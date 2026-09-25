import { describe, expect, it } from "vitest";
import type { ActionRequestWire, AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";
import { RIBBON_MAX_STRANDS, ribbonModel } from "./ribbon";

const at = "2026-09-24T10:00:00Z";
const task: AgentTask = { id: "t1", title: "Tidy Downloads", status: "running", created_at: at, updated_at: at };
const busy = { agent_busy: true, control_owner: "agent", model: "local" } as StatusPayload;
const quiet = { agent_busy: false, control_owner: "none", model: "local" } as StatusPayload;

const req = (id: string, type = "read_file", taskId = "t1"): ActionRequestWire =>
  ({ action_id: id, task_id: taskId, computer_id: "vm1", action: { type, path: "/tmp/a" }, requested_at: at });
const requested = (id: string, type?: string): AgentEvent => ({ type: "action_requested", request: req(id, type) });
const started = (id: string, type?: string): AgentEvent => ({ type: "action_started", request: req(id, type), action_id: id, at });
const completed = (id: string, outcome = "executed", success = true): AgentEvent =>
  ({ type: "action_completed", request: req(id), result: { action_id: id, success, outcome, message: "", duration_ms: 5 } });
const evaluated = (id: string, decision: string): AgentEvent =>
  ({ type: "action_evaluated", request: req(id), verdict: { decision, risk: "medium", reason: "writes outside the sandbox" } });

const model = (events: AgentEvent[], overrides: Partial<Parameters<typeof ribbonModel>[0]> = {}) =>
  ribbonModel({ task, events, status: busy, connected: true, ...overrides });

describe("ribbonModel: real events only", () => {
  it("is a trunk alone while the task runs with nothing in flight: no invented strands", () => {
    expect(model([])).toEqual({ trunk: "thinking", strands: [], activeId: undefined });
    expect(model([], { task: { ...task, status: "pending" } }).trunk).toBe("dormant");
    expect(model([], { connected: false }).trunk).toBe("dormant");
  });

  it("sprouts one strand per requested action, labelled from the action", () => {
    const m = model([requested("a1")]);
    expect(m.strands).toHaveLength(1);
    expect(m.strands[0]).toMatchObject({ id: "a1", capability: "Files", label: "Reading a file", phase: "sprouting" });
    expect(m.activeId).toBe("a1");
    expect(m.trunk).toBe("thinking");
  });

  it("runs only while Core corroborates input is busy", () => {
    expect(model([requested("a1"), started("a1")])).toMatchObject({ trunk: "working", activeId: "a1" });
    expect(model([requested("a1"), started("a1")]).strands[0].phase).toBe("running");
    expect(model([requested("a1"), started("a1")], { status: quiet }).strands[0].phase).toBe("sprouting");
    expect(model([started("a1")], { status: { ...busy, control_owner: "user" } })).toMatchObject({ trunk: "waiting" });
  });

  it("holds a strand at approval", () => {
    const pending = model([requested("a1"), evaluated("a1", "require_approval")]);
    expect(pending.strands[0].phase).toBe("approval");
    expect(pending.trunk).toBe("waiting");
    const waiting = model([requested("a2")], { task: { ...task, status: "waiting_for_approval" } });
    expect(waiting.strands[0].phase).toBe("approval");
    expect(waiting.trunk).toBe("waiting");
    expect(model([requested("a1"), completed("a1", "needs_approval", false)]).strands[0].phase).toBe("approval");
  });

  it("retracts completed strands and keeps failures visible", () => {
    const m = model([requested("a1"), started("a1"), completed("a1"), requested("a2"), started("a2"), completed("a2", "failed", false)]);
    expect(m.strands.map((s) => s.phase)).toEqual(["done", "failed"]);
    expect(m.activeId).toBeUndefined();
    expect(m.trunk).toBe("thinking");
    const denied: AgentEvent = { type: "action_denied", request: req("a3"), reason: "host path" };
    expect(model([denied]).strands[0].phase).toBe("blocked");
  });

  it("keeps only the four most recent strands and marks only the newest in-flight one active", () => {
    const events = ["a1", "a2", "a3", "a4", "a5"].flatMap((id) => [requested(id), completed(id)]);
    const m = model([...events, requested("a6")]);
    expect(m.strands).toHaveLength(RIBBON_MAX_STRANDS);
    expect(m.strands.map((s) => s.id)).toEqual(["a3", "a4", "a5", "a6"]);
    expect(m.activeId).toBe("a6");
  });

  it("settles when the task is done: nothing stays in flight", () => {
    const m = model([requested("a1"), completed("a1"), requested("a2")], { task: { ...task, status: "completed" } });
    expect(m.trunk).toBe("done");
    expect(m.activeId).toBeUndefined();
    expect(m.strands.map((s) => s.phase)).toEqual(["done", "interrupted"]);
    expect(model([], { task: { ...task, status: "failed" } }).trunk).toBe("dormant");
  });

  it("ignores other tasks' actions and never carries typed text", () => {
    const other: AgentEvent = { type: "action_requested", request: req("b1", "read_file", "t2") };
    const typing: AgentEvent = { type: "action_requested", request: { ...req("a1"), action: { type: "type_text", text: "hunter2" } } };
    const m = model([other, typing]);
    expect(m.strands.map((s) => s.id)).toEqual(["a1"]);
    expect(JSON.stringify(m)).not.toContain("hunter2");
  });
});

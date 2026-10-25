import { describe, expect, it, vi } from "vitest";
import type { AgentCursorAction } from "@pegoles/ui";
import type { AgentEvent } from "./tauri";
import { CursorEventMapper, agentCursorSource } from "./agentCursorFeed";

type Handler = (e: { payload: AgentEvent }) => void;
const { handlers } = vi.hoisted(() => ({ handlers: [] as Handler[] }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_: string, handler: Handler) => {
    handlers.push(handler);
    return Promise.resolve(() => undefined);
  }),
}));

let seq = 0;
function started(type: string, extra: Record<string, unknown> = {}, computer = "vm-1", task = "t1"): AgentEvent {
  seq += 1;
  const request = { action_id: `a${seq}`, task_id: task, computer_id: computer, action: { type, ...extra }, requested_at: "" };
  return { type: "action_started", action_id: request.action_id, request, at: "" } as AgentEvent;
}

function completed(start: AgentEvent): AgentEvent {
  const request = (start as { request: unknown }).request;
  return { type: "action_completed", request, result: {} } as unknown as AgentEvent;
}

describe("CursorEventMapper (real actions → agent cursor)", () => {
  it("maps each pointer action to its real normalized coordinate", () => {
    const m = new CursorEventMapper("vm-1");
    expect(m.map(started("move_pointer", { x: 0.2, y: 0.3 }))).toEqual([{ kind: "move", at: { x: 0.2, y: 0.3 } }]);
    expect(m.map(started("click", { x: 0.5, y: 0.5, button: "left" }))).toEqual([{ kind: "click", at: { x: 0.5, y: 0.5 }, count: 1 }]);
    expect(m.map(started("double_click", { x: 0.1, y: 0.9 }))).toEqual([{ kind: "click", at: { x: 0.1, y: 0.9 }, count: 2 }]);
    expect(m.map(started("mouse_down", { x: 0.4, y: 0.4 }))).toEqual([{ kind: "press", at: { x: 0.4, y: 0.4 } }]);
    expect(m.map(started("mouse_up", { x: 0.6, y: 0.4 }))).toEqual([{ kind: "release", at: { x: 0.6, y: 0.4 } }]);
    expect(m.map(started("drag", { from_x: 0.1, from_y: 0.2, to_x: 0.7, to_y: 0.8, duration_ms: 450 }))).toEqual([
      { kind: "drag", from: { x: 0.1, y: 0.2 }, to: { x: 0.7, y: 0.8 }, durationMs: 450 },
    ]);
    expect(m.map(started("scroll", { x: 0.5, y: 0.6, delta_x: 0, delta_y: -3 }))).toEqual([
      { kind: "scroll", at: { x: 0.5, y: 0.6 }, dx: 0, dy: -3 },
    ]);
  });

  it("shows looking, typing and waiting as states, and quiets down between actions", () => {
    const m = new CursorEventMapper("vm-1");
    const look = started("observe_screen");
    expect(m.map(look)).toEqual([{ kind: "observe" }]);
    expect(m.map(completed(look))).toEqual([{ kind: "think" }]);
    const type = started("type_text", { text: "hi" });
    expect(m.map(type)).toEqual([{ kind: "typing", active: true }]);
    expect(m.map(completed(type))).toEqual([{ kind: "typing", active: false }]);
    expect(m.map(started("key_press", { key: "Enter" }))).toEqual([{ kind: "typing", active: true }]);
    expect(m.map(started("wait", { ms: 500 }))).toEqual([{ kind: "think" }]);
  });

  it("ignores other computers, malformed coordinates and denied actions (never shows what did not happen)", () => {
    const m = new CursorEventMapper("vm-1");
    expect(m.map(started("click", { x: 0.5, y: 0.5 }, "vm-2"))).toEqual([]);
    expect(m.map(started("click", { x: "0.5", y: 0.5 }))).toEqual([]);
    expect(m.map(started("move_pointer", { x: Number.NaN, y: 0.5 }))).toEqual([]);
    expect(m.map(started("drag", { from_x: 0.1, from_y: 0.1, to_x: null, to_y: 0.2, duration_ms: 100 }))).toEqual([]);
    const request = { action_id: "x", task_id: "t1", computer_id: "vm-1", action: { type: "click", x: 0.9, y: 0.9 }, requested_at: "" };
    expect(m.map({ type: "action_requested", request } as AgentEvent)).toEqual([]);
    expect(m.map({ type: "action_denied", request, reason: "policy" } as AgentEvent)).toEqual([]);
  });

  it("settles when its task completes and stops when it is cancelled or fails", () => {
    const done = new CursorEventMapper("vm-1");
    done.map(started("click", { x: 0.5, y: 0.5 }));
    expect(done.map({ type: "task_status_changed", task_id: "other", from: "running", to: "completed", at: "" } as AgentEvent)).toEqual([]);
    expect(done.map({ type: "task_status_changed", task_id: "t1", from: "running", to: "completed", at: "" } as AgentEvent)).toEqual([{ kind: "done" }]);
    for (const to of ["cancelled", "failed"]) {
      const m = new CursorEventMapper("vm-1");
      m.map(started("move_pointer", { x: 0.5, y: 0.5 }));
      expect(m.map({ type: "task_status_changed", task_id: "t1", from: "running", to, at: "" } as AgentEvent)).toEqual([{ kind: "stop" }]);
    }
  });

  it("stops when the person takes control or the computer stops, pauses or disconnects", () => {
    const m = new CursorEventMapper("vm-1");
    const stop: AgentCursorAction[] = [{ kind: "stop" }];
    expect(m.map({ type: "control_ownership_changed", computer_id: "vm-1", from: "agent", to: "user", at: "" } as AgentEvent)).toEqual(stop);
    expect(m.map({ type: "control_ownership_changed", computer_id: "vm-1", from: "user", to: "agent", at: "" } as AgentEvent)).toEqual([]);
    for (const to of ["stopping", "stopped", "paused", "error"]) {
      expect(m.map({ type: "computer_state_changed", computer_id: "vm-1", from: "running", to, at: "" } as AgentEvent)).toEqual(stop);
    }
    expect(m.map({ type: "computer_state_changed", computer_id: "vm-2", from: "running", to: "stopped", at: "" } as AgentEvent)).toEqual([]);
    expect(m.map({ type: "guest_runtime_disconnected", computer_id: "vm-1", reason: "eof", at: "" } as AgentEvent)).toEqual(stop);
  });
});

describe("agentCursorSource", () => {
  it("relays live events to each subscriber from subscription on, never replaying history", () => {
    const source = agentCursorSource("vm-1");
    const early = started("click", { x: 0.1, y: 0.1 });
    const a: AgentCursorAction[] = [];
    const unsubscribe = source.subscribe((action) => a.push(action));
    expect(handlers.length).toBeGreaterThan(0);
    const emit = (payload: AgentEvent) => handlers.forEach((h) => h({ payload }));
    emit(started("move_pointer", { x: 0.3, y: 0.3 }));
    expect(a).toEqual([{ kind: "move", at: { x: 0.3, y: 0.3 } }]);
    const b: AgentCursorAction[] = [];
    source.subscribe((action) => b.push(action));
    expect(b).toEqual([]);
    unsubscribe();
    emit(started("move_pointer", { x: 0.6, y: 0.6 }));
    expect(a).toHaveLength(1);
    expect(b).toEqual([{ kind: "move", at: { x: 0.6, y: 0.6 } }]);
    expect(early).toBeTruthy();
  });

  it("a new computer id is a separate feed", () => {
    const old: AgentCursorAction[] = [];
    const next: AgentCursorAction[] = [];
    agentCursorSource("vm-1").subscribe((action) => old.push(action));
    agentCursorSource("vm-9").subscribe((action) => next.push(action));
    handlers.forEach((h) => h({ payload: started("click", { x: 0.5, y: 0.5 }, "vm-9", "t9") }));
    expect(old).toEqual([]);
    expect(next).toEqual([{ kind: "click", at: { x: 0.5, y: 0.5 }, count: 1 }]);
  });
});

import { describe, expect, it } from "vitest";
import { buildTranscript } from "./transcript";
import type { ActionRequestWire, AgentEvent, AgentTask } from "../lib/tauri";

const at = (s: number) => `2026-09-24T10:00:${String(s).padStart(2, "0")}Z`;
const task: AgentTask = { id: "t1", title: "Summarize the report", status: "running", created_at: at(0), updated_at: at(9) };
const req = (id: string, action: ActionRequestWire["action"], s: number): ActionRequestWire => ({ action_id: id, task_id: "t1", computer_id: "vm", action, requested_at: at(s) });
const completed = (request: ActionRequestWire, outcome = "executed"): AgentEvent => ({
  type: "action_completed", request, result: { action_id: request.action_id, success: outcome === "executed", outcome, message: "", duration_ms: 4 },
});

describe("buildTranscript", () => {
  it("starts with the request and shows nothing Core did not report", () => {
    expect(buildTranscript({ task, events: [] })).toEqual([{ kind: "request", id: "request-t1", text: task.title, at: at(0) }]);
  });

  it("gives files their own shape instead of an action row", () => {
    const click = req("a1", { type: "click" }, 1);
    const write = req("a2", { type: "write_file", path: "/home/pegoles/workspace/summary.md", content: "secret" }, 2);
    const items = buildTranscript({ task, events: [completed(click), completed(write)] });
    expect(items.map((item) => item.kind)).toEqual(["request", "actions", "file"]);
    expect(items[2]).toMatchObject({ kind: "file", path: "/home/pegoles/workspace/summary.md" });
    expect(JSON.stringify(items)).not.toContain("secret");
  });

  it("keeps in-flight actions out of history (they belong to Pegoles' turn)", () => {
    const click = req("a1", { type: "click" }, 1);
    const started: AgentEvent = { type: "action_started", action_id: "a1", request: click, at: at(1) };
    expect(buildTranscript({ task, events: [started] }).map((item) => item.kind)).toEqual(["request"]);
  });

  it("marks only the latest approval as open while the task waits", () => {
    const events: AgentEvent[] = [
      { type: "approval_requested", task_id: "t1", reason: "First", at: at(1) },
      { type: "approval_requested", task_id: "t1", reason: "Second", at: at(2) },
    ];
    const waiting = buildTranscript({ task: { ...task, status: "waiting_for_approval" }, events });
    expect(waiting.filter((item) => item.kind === "approval").map((item) => item.kind === "approval" && item.open)).toEqual([false, true]);
    const resumed = buildTranscript({ task, events });
    expect(resumed.some((item) => item.kind === "approval" && item.open)).toBe(false);
  });
});

describe("buildTranscript — work groups", () => {
  it("carries each action's verb and real duration, and marks consequential ones", () => {
    const look = req("a1", { type: "screenshot" }, 1);
    const run = req("a2", { type: "shell", command: "ls" }, 2);
    const items = buildTranscript({ task, events: [completed(look), completed(run)] });
    const group = items[1];
    expect(group.kind).toBe("actions");
    if (group.kind !== "actions") return;
    expect(group.steps.map((step) => [step.verb, step.durationMs, step.consequential])).toEqual([["screenshot", 4, false], ["shell", 4, true]]);
  });

  it("never invents a duration Core did not report", () => {
    const click = req("a1", { type: "click" }, 1);
    const denied: AgentEvent = { type: "action_denied", request: click, reason: "policy" };
    const items = buildTranscript({ task, events: [denied] });
    const group = items[1];
    expect(group.kind === "actions" && group.steps[0].durationMs).toBeUndefined();
  });

  it("closes a finished task with one outcome carrying real counts", () => {
    const click = req("a1", { type: "click" }, 1);
    const write = req("a2", { type: "write_file", path: "/w/a.md" }, 2);
    const done = { ...task, status: "completed" as const };
    const items = buildTranscript({ task: done, events: [completed(click), completed(write)] });
    expect(items[items.length - 1]).toEqual({ kind: "outcome", id: "outcome-t1", status: "completed", actions: 2, files: 1, at: at(9) });
    expect(buildTranscript({ task, events: [completed(click)] }).some((item) => item.kind === "outcome")).toBe(false);
  });
});

describe("buildTranscript — Pegoles' words", () => {
  const note = (kind: string, text: string, s: number, taskId = "t1"): AgentEvent =>
    ({ type: "agent_message", task_id: taskId, kind, text, at: at(s) }) as AgentEvent;

  it("tells notes, actions and the summary in the order they happened", () => {
    const look = req("a1", { type: "observe_screen" }, 2);
    const click = req("a2", { type: "click", x: 0.5, y: 0.5 }, 3);
    const typed = req("a3", { type: "type_text", text: "", sensitive: false }, 6);
    const events = [
      note("progress", "Opening the browser", 1), completed(look), completed(click),
      note("progress", "Typing the search", 5), completed(typed),
      note("summary", "Found three plans.", 8),
    ];
    const items = buildTranscript({ task: { ...task, status: "completed" }, events });
    expect(items.map((item) => item.kind === "message" ? `${item.kind}:${item.variant}` : item.kind))
      .toEqual(["request", "message:progress", "actions", "message:progress", "actions", "message:summary", "outcome"]);
    // A note ends the run before it: each run is what Pegoles did after saying what it would do.
    const runs = items.filter((item) => item.kind === "actions");
    expect(runs.map((run) => run.kind === "actions" && run.steps.map((step) => step.id))).toEqual([["a1", "a2"], ["a3"]]);
    expect(items[items.length - 2]).toMatchObject({ kind: "message", variant: "summary", text: "Found three plans." });
  });

  it("puts a note before the action it introduced when both carry the same instant", () => {
    const click = req("a1", { type: "click", x: 0.1, y: 0.1 }, 4);
    const items = buildTranscript({ task, events: [completed(click), note("progress", "Clicking Sign in", 4)] });
    expect(items.map((item) => item.kind)).toEqual(["request", "message", "actions"]);
  });

  it("keeps the text as Core sent it (trimmed), and drops empty, malformed or foreign notes", () => {
    const events = [
      note("progress", "  <b>not markup</b>\n", 1),
      note("progress", "   ", 2),
      note("shout", "unknown kind", 3),
      note("progress", "another task", 4, "t2"),
      { type: "agent_message", task_id: "t1", kind: "progress", at: at(5) } as unknown as AgentEvent,
    ];
    const messages = buildTranscript({ task, events }).filter((item) => item.kind === "message");
    expect(messages).toEqual([{ kind: "message", id: `message-0-${at(1)}`, variant: "progress", text: "<b>not markup</b>", at: at(1) }]);
  });

  it("says why a run stopped before how it ended", () => {
    const items = buildTranscript({ task: { ...task, status: "cancelled" }, events: [note("error", "Stopped by the user.", 3)] });
    expect(items.slice(-2)).toMatchObject([{ kind: "message", variant: "error", text: "Stopped by the user." }, { kind: "outcome", status: "cancelled" }]);
  });
});

import { describe, expect, it } from "vitest";
import type { AgentEvent, TeamStatus, TeamStepId } from "../lib/tauri";
import { MAX_SUMMARY_CHARS, modeFor, teamEventOf, teamRunOf } from "./teamRun";

function must<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error("expected a value");
  return value;
}
const at = "2026-10-25T10:00:00Z";
const ev = (step: TeamStepId, status: TeamStatus, summary = ""): AgentEvent => ({ type: "team_progress", task_id: "t", step, status, summary, at });

describe("teamRunOf", () => {
  it("is null for a task that ran with one model", () => {
    expect(teamRunOf([])).toBeNull();
    expect(teamRunOf([{ type: "agent_message", task_id: "t", kind: "progress", text: "x", at }])).toBeNull();
  });

  it("starts with every step pending and follows real events in order", () => {
    const run = must(teamRunOf([ev("captain_plan", "working")]));
    expect(run.steps.map((s) => s.state)).toEqual(["working", "pending", "pending", "pending", "pending"]);
    expect(run.active).toBe("captain_plan");
    expect(run.settled).toBe(false);
    expect(run.roles).toEqual({ captain: "working", researcher: "pending", executor: "pending", critic: "pending" });
  });

  it("settles after the captain's synthesis and keeps each summary", () => {
    const run = must(teamRunOf([
      ev("captain_plan", "working"), ev("captain_plan", "done", "1. open"), ev("research", "working"), ev("research", "done", "idle"),
      ev("execute", "working"), ev("execute", "done", "typed"), ev("critique", "working"), ev("critique", "done", "PASS: ok"),
      ev("synthesis", "working"), ev("synthesis", "done", "All set."),
    ]));
    expect(run.settled).toBe(true);
    expect(run.failed).toBe(false);
    expect(run.active).toBeNull();
    expect(run.steps.map((s) => s.summary)).toEqual(["1. open", "idle", "typed", "PASS: ok", "All set."]);
    expect(run.roles.captain).toBe("done");
  });

  it("shows a failure and the steps that never ran as skipped", () => {
    const run = must(teamRunOf([ev("captain_plan", "done", "p"), ev("research", "failed"), ev("execute", "skipped"), ev("critique", "skipped"), ev("synthesis", "skipped")]));
    expect(run.failed).toBe(true);
    expect(run.settled).toBe(true);
    expect(run.roles.researcher).toBe("failed");
    expect(run.roles.captain).toBe("skipped");
  });

  it("clears the steps after one that works again (the old critic and answer are stale)", () => {
    const run = must(teamRunOf([
      ev("execute", "done", "first"), ev("critique", "done", "PASS"), ev("synthesis", "done", "first answer"),
      ev("execute", "working"),
    ]));
    expect(run.steps.map((s) => s.state)).toEqual(["pending", "pending", "working", "pending", "pending"]);
    expect(run.steps[4].summary).toBe("");
    expect(run.settled).toBe(false);
  });

  it("ignores malformed events and bounds hostile summaries without interpreting them", () => {
    expect(teamEventOf({ type: "team_progress", step: "boss", status: "done", summary: "", task_id: "t", at })).toBeNull();
    expect(teamEventOf({ type: "team_progress", step: "execute", status: "winning", summary: "", task_id: "t", at })).toBeNull();
    const hostile = `<img src=x onerror=alert(1)>${"x".repeat(5000)}`;
    const parsed = must(teamEventOf(ev("execute", "done", hostile)));
    expect([...parsed.summary].length).toBeLessThanOrEqual(MAX_SUMMARY_CHARS);
    expect(parsed.summary.startsWith("<img src=x")).toBe(true);
    expect(teamEventOf({ type: "team_progress", step: "execute", status: "done", summary: 7, task_id: "t", at } as unknown as AgentEvent)?.summary).toBe("");
  });

  it("maps only real states to avatar modes", () => {
    expect(modeFor("working")).toBe("working");
    expect(modeFor("done")).toBe("done");
    expect(modeFor("failed")).toBe("error");
    expect(modeFor("pending")).toBe("idle");
    expect(modeFor("skipped")).toBe("idle");
  });
});

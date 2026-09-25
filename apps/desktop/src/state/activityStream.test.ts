import { describe, expect, it } from "vitest";
import { activityStream, activitySummary, filterDays, tasksWithoutActivity } from "./activityStream";
import type { AgentEvent, AgentTask } from "../lib/tauri";

const req = (id: string, type: string, extra: object = {}, taskId: string | null = null, at = "2026-09-24T10:00:00Z") =>
  ({ action_id: id, task_id: taskId, computer_id: "vm", action: { type, ...extra }, requested_at: at });
const done = (id: string, type: string, extra: object = {}, taskId: string | null = null, at = "2026-09-24T10:00:00Z") => [
  { type: "action_started", action_id: id, request: req(id, type, extra, taskId, at), at },
  { type: "action_completed", request: req(id, type, extra, taskId, at), result: { action_id: id, outcome: "executed", success: true, message: "", duration_ms: 16 } },
];
const now = new Date("2026-09-24T18:00:00Z");
const task = (id: string, title: string, status: AgentTask["status"]): AgentTask =>
  ({ id, title, status, created_at: "2026-09-24T10:05:00Z", updated_at: "2026-09-24T10:05:00Z" });
const tasks: AgentTask[] = [task("t1", "Later task", "pending")];

describe("activityStream", () => {
  it("orders newest first, collapses an action to one past-tense sentence and hides pointer moves", () => {
    const events = [
      { type: "action_requested", request: req("m1", "move_pointer", { x: 0.1, y: 0.1 }) },
      { type: "action_requested", request: req("a1", "click", { x: 0.5, y: 0.5 }) },
      { type: "action_completed", request: req("a1", "click"), result: { action_id: "a1", outcome: "executed", success: true, message: "", duration_ms: 16, completed_at: "2026-09-24T10:00:01Z" } },
      { type: "task_created", task_id: "t1", title: "Later task", at: "2026-09-24T10:05:00Z" },
    ] as unknown as AgentEvent[];
    const [today] = activityStream(events, tasks, now);
    expect(today.label).toBe("Today");
    expect(today.entries.map((entry) => entry.text)).toEqual(["You handed this over", "Clicked"]);
    expect(today.entries.some((entry) => entry.current)).toBe(false);
  });

  it("marks an action still in flight as current", () => {
    const [today] = activityStream([{ type: "action_requested", request: req("a2", "scroll") }] as unknown as AgentEvent[], [], now);
    expect(today.entries[0]).toMatchObject({ text: "Scrolling…", current: true, outcome: "running" });
  });

  it("speaks about control and failures like a person, and never leaks typed text", () => {
    const events = [
      { type: "control_ownership_changed", computer_id: "vm", from: "agent", to: "user", at: "2026-09-24T11:00:00Z" },
      { type: "action_denied", request: req("a3", "type_text", { text: "hunter2" }), reason: "credential" },
    ] as unknown as AgentEvent[];
    const [today] = activityStream(events, [], now);
    const texts = today.entries.map((entry) => entry.text);
    expect(texts).toContain("You took control of its computer");
    expect(texts).toContain("Typing — blocked by policy");
    expect(JSON.stringify(today)).not.toContain("hunter2");
  });

  it("leaves boot handshakes out and groups older days", () => {
    const events = [
      { type: "guest_handshake_completed", computer_id: "vm", protocol_version: 2, at: "2026-09-24T09:00:00Z" },
      { type: "computer_state_changed", computer_id: "vm", from: "starting", to: "running", at: "2026-09-23T09:00:00Z" },
    ] as unknown as AgentEvent[];
    const days = activityStream(events, [], now);
    expect(days.map((day) => day.label)).toEqual(["Yesterday"]);
    expect(days[0].entries[0].text).toBe("Started its computer");
  });

  it("shows file names and hosts instead of full paths, keeping the path in details", () => {
    const events = [
      ...done("f1", "read_file", { path: "/home/pegoles/workspace/Downloads/index.txt" }, "t1", "2026-09-24T10:01:00Z"),
      ...done("f2", "open_url", { url: "https://www.tesla.com/models" }, "t1", "2026-09-24T10:02:00Z"),
    ] as unknown as AgentEvent[];
    const [today] = activityStream(events, tasks, now);
    const [read, visit] = today.groups[0].entries;
    expect(read).toMatchObject({ text: "Read index.txt", context: "Downloads", outcome: "done" });
    expect(read.details).toContain("/home/pegoles/workspace/Downloads/index.txt");
    expect(visit).toMatchObject({ text: "Visited tesla.com", context: null });
  });

  it("describes actions that did not run in the present, never as done", () => {
    const events = [
      { type: "action_started", action_id: "p1", request: req("p1", "open_url", { url: "https://www.dropbox.com/home" }, "t1"), at: "2026-09-24T10:00:00Z" },
      { type: "action_completed", request: req("p1", "open_url", { url: "https://www.dropbox.com/home" }, "t1"), result: { action_id: "p1", outcome: "needs_approval", success: false, message: "", duration_ms: 3 } },
    ] as unknown as AgentEvent[];
    const [today] = activityStream(events, tasks, now);
    expect(today.entries[0]).toMatchObject({ text: "Opening dropbox.com — needs approval", outcome: "attention" });
  });
});

describe("activity groups", () => {
  const mixed = [
    { type: "computer_state_changed", computer_id: "vm", from: "starting", to: "running", at: "2026-09-24T09:00:00Z" },
    { type: "guest_runtime_ready", computer_id: "vm", ready_in_ms: 4200, at: "2026-09-24T09:00:05Z" },
    { type: "task_created", task_id: "t1", title: "Research competitors", at: "2026-09-24T09:10:00Z" },
    ...done("c1", "open_url", { url: "https://tesla.com" }, "t1", "2026-09-24T09:11:00Z"),
    ...done("c2", "open_url", { url: "https://byd.com" }, "t1", "2026-09-24T09:12:00Z"),
    { type: "task_created", task_id: "t2", title: "Organize Downloads", at: "2026-09-24T09:20:00Z" },
    ...done("c3", "list_directory", { path: "/home/pegoles/workspace/Downloads" }, "t2", "2026-09-24T09:21:00Z"),
    ...done("c4", "write_file", { path: "/home/pegoles/workspace/comparison.md", content: "secret body" }, "t1", "2026-09-24T09:30:00Z"),
  ] as unknown as AgentEvent[];
  const world = [task("t1", "Research competitors", "running"), task("t2", "Organize Downloads", "waiting_for_approval"), task("t3", "Rename photos", "completed")];

  it("groups consecutive entries of a task into runs, newest run first, steps in order", () => {
    const [today] = activityStream(mixed, world, now);
    expect(today.groups.map((group) => group.title)).toEqual(["Research competitors", "Organize Downloads", "Research competitors", "Pegoles Computer"]);
    const [latest, , earlier, computer] = today.groups;
    expect(latest.entries.map((entry) => entry.text)).toEqual(["Wrote comparison.md"]);
    expect(earlier.entries.map((entry) => entry.text)).toEqual(["You handed this over", "Visited tesla.com", "Visited byd.com"]);
    expect(earlier.at).toBe("2026-09-24T09:10:00Z");
    expect(computer).toMatchObject({ kind: "computer", taskId: null, status: null });
    expect(computer.entries.map((entry) => entry.text)).toEqual(["Started its computer", "Its computer is ready · 4.2 s"]);
    expect(JSON.stringify(today)).not.toContain("secret body");
  });

  it("puts the real task status on the task's newest run only", () => {
    const [today] = activityStream(mixed, world, now);
    const [latest, organize, earlier] = today.groups;
    expect(latest).toMatchObject({ status: "running", statusLabel: "Working", statusTone: "active" });
    expect(organize).toMatchObject({ status: "waiting_for_approval", statusLabel: "Needs you", statusTone: "attention" });
    expect(earlier.status).toBeNull();
  });

  it("labels every task status in plain words", () => {
    const statuses: AgentTask["status"][] = ["pending", "completed", "failed", "cancelled"];
    const labels = statuses.map((status) => {
      const [today] = activityStream([{ type: "task_created", task_id: "x", title: "X", at: "2026-09-24T09:10:00Z" }] as unknown as AgentEvent[], [task("x", "X", status)], now);
      return today.groups[0].statusLabel;
    });
    expect(labels).toEqual(["Not started", "Done", "Couldn’t finish", "Cancelled"]);
  });

  it("summarises today from real counts", () => {
    const days = activityStream(mixed, world, now);
    expect(activitySummary(days, world, now)).toEqual({ scope: "today", tasks: 2, actions: 4, needsYou: 1 });
    expect(activitySummary([], [], now)).toEqual({ scope: "none", tasks: 0, actions: 0, needsYou: 0 });
  });

  it("filters to what needs you or to the computer", () => {
    const days = activityStream(mixed, world, now);
    expect(filterDays(days, "needs-you")[0].groups.map((group) => group.title)).toEqual(["Organize Downloads"]);
    expect(filterDays(days, "computer")[0].groups.map((group) => group.title)).toEqual(["Pegoles Computer"]);
    expect(filterDays(days, "all")).toBe(days);
    expect(filterDays(activityStream(mixed, [], now), "needs-you")).toEqual([]);
  });

  it("lists tasks that have no activity yet", () => {
    const days = activityStream(mixed, world, now);
    expect(tasksWithoutActivity(days, world).map((item) => item.id)).toEqual(["t3"]);
  });
});

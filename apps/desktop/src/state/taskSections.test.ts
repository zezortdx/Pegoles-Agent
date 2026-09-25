import { describe, expect, it } from "vitest";
import { taskSections } from "./taskSections";
import type { AgentTask } from "../lib/tauri";

const NOW = new Date("2026-09-26T15:00:00").getTime();
const at = (hoursAgo: number) => new Date(NOW - hoursAgo * 3_600_000).toISOString();
const task = (id: string, status: AgentTask["status"], hoursAgo: number): AgentTask =>
  ({ id, title: id, status, created_at: at(hoursAgo + 1), updated_at: at(hoursAgo) });

describe("taskSections", () => {
  it("lifts attention and live work above time groups", () => {
    const sections = taskSections([
      task("done-today", "completed", 1),
      task("waiting", "waiting_for_approval", 30),
      task("live", "running", 2),
    ], NOW);
    expect(sections.map((section) => section.key)).toEqual(["needs-you", "working", "today"]);
    expect(sections[0].tasks[0].id).toBe("waiting");
  });

  it("files settled and pending tasks by when they last changed, newest first", () => {
    const sections = taskSections([
      task("old", "completed", 24 * 20),
      task("yesterday", "failed", 24 + 2),
      task("week", "pending", 24 * 4),
      task("a", "completed", 3),
      task("b", "pending", 1),
    ], NOW);
    expect(sections.map((section) => section.label)).toEqual(["Today", "Yesterday", "Previous 7 days", "Earlier"]);
    expect(sections[0].tasks.map((item) => item.id)).toEqual(["b", "a"]);
  });

  it("returns nothing for no tasks", () => {
    expect(taskSections([], NOW)).toEqual([]);
  });
});

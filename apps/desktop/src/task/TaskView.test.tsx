import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TaskView } from "./TaskView";
import { StatusDock } from "./StatusDock";
import type { TaskActivity } from "../state/agentState";
import type { TranscriptItem, TranscriptStep } from "../state/transcript";
import { activityPill } from "../lib/taskState";
import type { AgentTask } from "../lib/tauri";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const at = "2026-09-24T10:00:00Z";
const task: AgentTask = { id: "t", title: "Research competitors", status: "pending", created_at: at, updated_at: at };
const request: TranscriptItem = { kind: "request", id: "r", text: task.title, at };
const quiet: TaskActivity = { mode: "blocked", headline: "Can’t start yet", detail: "No model is connected in this build.", recent: [], pulse: 0, live: true, offerComputer: true };
const working: TaskActivity = { mode: "working", headline: "Working with files", recent: [], pulse: 1, live: true };

const view = (activity: TaskActivity, items: readonly TranscriptItem[] = [request], patch: Partial<AgentTask> = {}, handlers: { onOpenComputer?: () => void; onHeadingVisible?: (visible: boolean) => void } = {}) => {
  const current = { ...task, ...patch };
  return render(
    <TaskView
      task={current}
      items={items}
      activity={activity}
      state={activityPill(activity.mode, current.status)}
      animated={false}
      arriving={false}
      onHeadingVisible={handlers.onHeadingVisible ?? (() => undefined)}
      onOpenComputer={handlers.onOpenComputer ?? (() => undefined)}
      onModelSettings={() => undefined}
    />,
  );
};

const dock = (activity: TaskActivity, patch: Partial<Parameters<typeof StatusDock>[0]> = {}) => render(
  <StatusDock activity={activity} state={activityPill(activity.mode, activity.live ? "running" : "completed")} arriving={false}
    interruptible={false} interrupting={false} computerOpen={false} modifier="⌘"
    onInterrupt={() => undefined} onWatch={() => undefined} onNewTask={() => undefined} {...patch} />,
);

const step = (id: string, label: string, verb: string, consequential: boolean, durationMs: number | undefined = 120): TranscriptStep =>
  ({ id, label, verb, consequential, outcome: "done", at, capability: consequential ? "Shell" : "Computer", durationMs });

describe("TaskView", () => {
  it("reads as a work session: the objective is the heading, the state and facts sit under it", () => {
    view(quiet);
    expect(screen.getByRole("heading", { level: 1, name: "Research competitors" })).toBeTruthy();
    expect(screen.getByText("Not started", { selector: ".state-label" })).toBeTruthy();
    expect(screen.getByText(/^Created /)).toBeTruthy();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("says once why it can't start, with the real ways forward", () => {
    const open = vi.fn();
    view(quiet, [request], {}, { onOpenComputer: open });
    expect(screen.getByText("Waiting for a model")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open its computer" }));
    expect(open).toHaveBeenCalledOnce();
  });

  it("shows an honest empty state before any work has happened", () => {
    view(working, [request], { status: "running" });
    expect(screen.getByText(/Waiting for the first action/)).toBeTruthy();
    expect(screen.queryByRole("list", { name: "Task timeline" })).toBeNull();
  });

  it("shows an approval where the run stopped, without inventing an answer", () => {
    const items: TranscriptItem[] = [request, { kind: "approval", id: "p", reason: "Open example.com", open: true, at }];
    const waiting: TaskActivity = { mode: "needs-user", headline: "Needs your approval", approvalReason: "Open example.com", recent: [], pulse: 1, live: true };
    view(waiting, items, { status: "waiting_for_approval" });
    expect(screen.getByText("Stopped to ask you")).toBeTruthy();
    expect(screen.getByText(/Nothing has been allowed/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Allow|Approve/ })).toBeNull();
  });

  it("gives a written file its own shape", () => {
    view(working, [request, { kind: "file", id: "f", path: "/home/pegoles/workspace/report.md", at }], { status: "running" });
    expect(screen.getByRole("article", { name: "File created: report.md" })).toBeTruthy();
  });

  it("folds a finished run into one sentence, but never hides what changed something", () => {
    const steps = [step("s1", "Looked at the screen", "screenshot", false), step("s2", "Clicked", "click", false), step("s3", "Ran a command", "shell", true)];
    const done: TaskActivity = { mode: "done", headline: "Done", recent: [], pulse: 3, live: false, at };
    view(done, [request, { kind: "actions", id: "g", steps, at }], { status: "completed" });
    const summary = screen.getByRole("button", { name: /Looked at the screen, clicked and ran a command/ });
    expect(summary.textContent).toContain("3 actions · <1s");
    expect(summary.getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByText("Ran a command", { selector: ".step__label" })).toBeTruthy();
    expect(screen.queryByText("Clicked", { selector: ".step__label" })).toBeNull();
    fireEvent.click(summary);
    expect(summary.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Clicked", { selector: ".step__label" })).toBeTruthy();
  });

  it("keeps the run Pegoles is in the middle of open", () => {
    const steps = [step("s1", "Clicked", "click", false), step("s2", "Scrolled", "scroll", false)];
    view(working, [request, { kind: "actions", id: "g", steps, at }], { status: "running" });
    expect(screen.getByRole("button", { name: /Clicked and scrolled/ }).getAttribute("aria-expanded")).toBe("true");
  });

  it("only states a run's duration when Core measured every action", () => {
    const steps = [step("s1", "Clicked", "click", false, 50), { ...step("s2", "Scrolled", "scroll", false), durationMs: undefined }];
    const done: TaskActivity = { mode: "done", headline: "Done", recent: [], pulse: 2, live: false, at };
    view(done, [request, { kind: "actions", id: "g", steps, at }], { status: "completed" });
    const meta = screen.getByText(/2 actions/, { selector: ".run__meta" });
    expect(meta.textContent).toBe("2 actions");
  });

  it("ends a settled task on how it ended", () => {
    const done: TaskActivity = { mode: "done", headline: "Done", recent: [], pulse: 4, live: false, at };
    view(done, [request, { kind: "file", id: "f", path: "/w/report.md", at }, { kind: "outcome", id: "o", status: "completed", actions: 4, files: 1, at }], { status: "completed" });
    const result = screen.getByRole("region", { name: "Finished" });
    expect(result.textContent).toContain("Done");
    expect(result.textContent).toMatch(/4 actions · 1 file · finished/);
  });

  it("tells the toolbar when the objective scrolls out of view", () => {
    type Notify = (entries: { isIntersecting: boolean }[]) => void;
    const watchers = new Map<Element, Notify>();
    class FakeObserver {
      constructor(private readonly callback: Notify) {}
      observe(element: Element) { watchers.set(element, this.callback); }
      unobserve(element: Element) { watchers.delete(element); }
      disconnect() { /* nothing held */ }
    }
    vi.stubGlobal("IntersectionObserver", FakeObserver);
    const onHeadingVisible = vi.fn();
    view(quiet, [request], {}, { onHeadingVisible });
    const notify = [...watchers.values()][0];
    expect(notify).toBeTruthy();
    notify?.([{ isIntersecting: false }]);
    expect(onHeadingVisible).toHaveBeenLastCalledWith(false);
  });
});

describe("StatusDock", () => {
  it("says what Pegoles is doing now and offers only real interventions", () => {
    const onInterrupt = vi.fn();
    dock({ ...working, detail: "/home/pegoles/workspace/notes.md" }, { interruptible: true, onInterrupt });
    const now = screen.getByRole("region", { name: "Now" });
    expect(now.textContent).toContain("Working with files");
    expect(now.textContent).toContain("notes.md");
    fireEvent.click(screen.getByRole("button", { name: /Stop/ }));
    expect(onInterrupt).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: /New task/ })).toBeNull();
  });

  it("offers to watch while Pegoles uses its computer out of sight", () => {
    const onWatch = vi.fn();
    dock({ mode: "using-computer", headline: "Using its computer", recent: [], pulse: 1, live: true }, { onWatch });
    fireEvent.click(screen.getByRole("button", { name: /Watch/ }));
    expect(onWatch).toHaveBeenCalledOnce();
  });

  it("offers the next job once the work has settled", () => {
    dock({ mode: "done", headline: "Done", recent: [], pulse: 4, live: false, at });
    expect(screen.getByRole("region", { name: "Now" }).textContent).toMatch(/Finished/);
    expect(screen.getByRole("button", { name: /New task/ })).toBeTruthy();
  });
});

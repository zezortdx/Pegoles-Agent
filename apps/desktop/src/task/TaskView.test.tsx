import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TaskView } from "./TaskView";
import { StatusDock } from "./StatusDock";
import type { TaskActivity } from "../state/agentState";
import type { TranscriptItem, TranscriptStep } from "../state/transcript";
import { activityPill } from "../lib/taskState";
import type { AgentTask } from "../lib/tauri";
import type { HumanError } from "../state/errors";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const at = "2026-09-24T10:00:00Z";
const task: AgentTask = { id: "t", title: "Research competitors", status: "pending", created_at: at, updated_at: at };
const request: TranscriptItem = { kind: "request", id: "r", text: task.title, at };
const quiet: TaskActivity = { mode: "blocked", headline: "Can’t start yet", detail: "Pegoles needs a model to work on tasks.", recent: [], pulse: 0, live: true, offerComputer: true, start: "needs-model" };
const ready: TaskActivity = { mode: "idle", headline: "Not started", recent: [], pulse: 0, live: false, start: "ready" };
const working: TaskActivity = { mode: "working", headline: "Working with files", recent: [], pulse: 1, live: true };

interface Extra {
  onOpenComputer?: () => void;
  onHeadingVisible?: (visible: boolean) => void;
  onStart?: () => void;
  starting?: boolean;
  startError?: HumanError | null;
}

const view = (activity: TaskActivity, items: readonly TranscriptItem[] = [request], patch: Partial<AgentTask> = {}, handlers: Extra = {}) => {
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
      onStart={handlers.onStart ?? (() => undefined)}
      starting={handlers.starting}
      startError={handlers.startError}
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

  it("offers Start once a model is connected, and says why a start didn't happen", () => {
    const onStart = vi.fn();
    const refused: HumanError = { scope: "run", title: "Pegoles is working on another task.", hint: "It works on one task at a time.", detail: "task x is already running", retryable: true };
    view(ready, [request], {}, { onStart, startError: refused });
    expect(screen.getByText("Not started yet")).toBeTruthy();
    expect(screen.queryByText("Waiting for a model")).toBeNull();
    expect(screen.getByRole("alert").textContent).toBe("Pegoles is working on another task. It works on one task at a time.");
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(onStart).toHaveBeenCalledOnce();
  });

  it("holds Start while Pegoles works on another task, or while a start is in flight", () => {
    const view1 = view({ ...ready, start: "busy", detail: "Pegoles is working on another task" });
    expect(screen.getByText("Pegoles is busy")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Start" }) as HTMLButtonElement).disabled).toBe(true);
    view1.unmount();
    view(ready, [request], {}, { starting: true });
    expect((screen.getByRole("button", { name: /Starting/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("never offers Start without a model", () => {
    view(quiet);
    expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
    expect(screen.getByRole("button", { name: "Model settings" })).toBeTruthy();
  });

  it("tells Pegoles' own words as plain text, never markup", () => {
    const items: TranscriptItem[] = [
      request,
      { kind: "message", id: "m1", variant: "progress", text: "Opening <b>the</b> <img src=x onerror=alert(1)> browser", at },
      { kind: "message", id: "m2", variant: "summary", text: "Found three plans.\nThe cheapest is Free.", at },
      { kind: "message", id: "m3", variant: "error", text: "Stopped by the user.", at },
    ];
    view({ ...working, live: false, mode: "idle" }, items, { status: "cancelled" });
    const note = screen.getByRole("article", { name: "Note from Pegoles" });
    expect(note.textContent).toBe("Opening <b>the</b> <img src=x onerror=alert(1)> browser");
    expect(note.querySelector("b, img")).toBeNull();
    expect(screen.getByRole("article", { name: "Pegoles’ summary" }).textContent).toContain("The cheapest is Free.");
    expect(screen.getByRole("article", { name: "Why Pegoles stopped" }).textContent).toBe("Stopped by the user.");
  });

  it("folds a long note until asked", () => {
    const long = Array.from({ length: 12 }, (_, i) => `Line ${i + 1} of what Pegoles found.`).join("\n");
    view(working, [request, { kind: "message", id: "m", variant: "summary", text: long, at }], { status: "running" });
    const text = screen.getByText(/Line 1 of what/);
    expect(text.hasAttribute("data-folded")).toBe(true);
    const more = screen.getByRole("button", { name: "Show more" });
    expect(more.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(more);
    expect(text.hasAttribute("data-folded")).toBe(false);
    expect(screen.getByRole("button", { name: "Show less" }).getAttribute("aria-expanded")).toBe("true");
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

  it("says why a stopped run ended, in Pegoles' words", () => {
    dock({ mode: "idle", headline: "Cancelled", recent: [], pulse: 2, live: false, at, reason: "Stopped by the user." });
    expect(screen.getByRole("region", { name: "Now" }).textContent).toContain("Stopped by the user.");
  });

  it("says a task hasn't started, and why, without offering Stop", () => {
    dock({ mode: "idle", headline: "Not started", detail: "Pegoles is working on another task", recent: [], pulse: 0, live: false, start: "busy" });
    const now = screen.getByRole("region", { name: "Now" });
    expect(now.textContent).toContain("Not started");
    expect(now.textContent).toContain("Pegoles is working on another task");
    expect(screen.queryByRole("button", { name: /Stop/ })).toBeNull();
  });

  it("offers the next job once the work has settled", () => {
    dock({ mode: "done", headline: "Done", recent: [], pulse: 4, live: false, at });
    expect(screen.getByRole("region", { name: "Now" }).textContent).toMatch(/Finished/);
    expect(screen.getByRole("button", { name: /New task/ })).toBeTruthy();
  });
});

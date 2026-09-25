import type { AgentTask } from "./tauri";

/**
 * One vocabulary for a task's state, wherever it appears (sidebar, title
 * bar, brief, Home, palette). Tone carries meaning: live is Pegoles acting
 * (blue light), attention is you (amber), done green, error red, quiet grey.
 */
export type StateTone = "live" | "attention" | "error" | "done" | "quiet";

export interface TaskState {
  readonly label: string;
  readonly tone: StateTone;
  /** Where the task sits in the sidebar and on Home. */
  readonly group: "needs-you" | "working" | "waiting" | "finished";
}

/** `modelReady`: a model is connected, so a pending task is queued rather than stuck. */
export function taskState(task: AgentTask, modelReady: boolean): TaskState {
  switch (task.status) {
    case "running": return { label: "Working", tone: "live", group: "working" };
    case "waiting_for_approval": return { label: "Needs you", tone: "attention", group: "needs-you" };
    case "failed": return { label: "Couldn’t finish", tone: "error", group: "finished" };
    case "completed": return { label: "Done", tone: "done", group: "finished" };
    case "cancelled": return { label: "Cancelled", tone: "quiet", group: "finished" };
    default: return { label: modelReady ? "Queued" : "Not started", tone: "quiet", group: "waiting" };
  }
}

export const GROUP_ORDER = ["needs-you", "working", "waiting", "finished"] as const;
export type TaskGroup = (typeof GROUP_ORDER)[number];

export const GROUP_LABEL: Record<TaskGroup, string> = {
  "needs-you": "Needs you",
  working: "Working",
  waiting: "Not started",
  finished: "Done",
};

/** Presence modes in which Pegoles is really doing something right now. */
const ACTING = new Set(["thinking", "planning", "working", "using-computer", "acknowledging"]);

export interface ActivityPill {
  readonly label: string;
  readonly tone: StateTone;
  readonly live: boolean;
}

/**
 * The state of the task in focus, from what Pegoles is doing (not only the
 * task record): while you hold its computer it says so, instead of
 * "Working" over a machine nobody else is touching.
 */
export function activityPill(mode: string, status: AgentTask["status"]): ActivityPill {
  if (mode === "needs-user") return { label: "Needs you", tone: "attention", live: false };
  if (mode === "waiting" && status !== "pending") return { label: "You have control", tone: "attention", live: false };
  if (ACTING.has(mode)) return { label: "Working", tone: "live", live: true };
  if (mode === "done") return { label: "Done", tone: "done", live: false };
  if (mode === "error") return status === "failed" ? { label: "Couldn’t finish", tone: "error", live: false } : { label: "Stuck", tone: "error", live: false };
  if (mode === "blocked") return status === "pending" ? { label: "Not started", tone: "quiet", live: false } : { label: "Blocked", tone: "attention", live: false };
  if (status === "cancelled") return { label: "Cancelled", tone: "quiet", live: false };
  if (mode === "offline") return { label: "Offline", tone: "quiet", live: false };
  return { label: "Queued", tone: "quiet", live: false };
}

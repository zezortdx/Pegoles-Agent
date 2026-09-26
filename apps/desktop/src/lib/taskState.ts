import type { AgentTask } from "./tauri";

/**
 * One vocabulary for a task's state, wherever it appears (status bar, title
 * bar, task heading). Tone carries meaning: live is Pegoles acting (blue
 * light), attention is you (amber), done green, error red, quiet grey.
 */
export type StateTone = "live" | "attention" | "error" | "done" | "quiet";

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
  if (mode === "offline") return { label: "Offline", tone: "quiet", live: false };
  // Nothing starts a task but a run: a pending task hasn't started, whatever the reason.
  if (status === "pending") return { label: "Not started", tone: "quiet", live: false };
  if (mode === "blocked") return { label: "Blocked", tone: "attention", live: false };
  if (status === "cancelled") return { label: "Cancelled", tone: "quiet", live: false };
  return { label: "Idle", tone: "quiet", live: false };
}

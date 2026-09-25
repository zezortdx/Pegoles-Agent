/**
 * Every state the Pegoles Presence can express. The shell maps real Core
 * signals onto these (see docs/FRONTEND_EXPERIENCE.md §3); the presence
 * itself never decides what Pegoles is doing.
 */
export type PresenceMode =
  | "offline"
  | "idle"
  | "attentive"
  | "acknowledging"
  | "thinking"
  | "planning"
  | "working"
  | "using-computer"
  | "waiting"
  | "needs-user"
  | "blocked"
  | "done"
  | "error";

export const PRESENCE_MODES: readonly PresenceMode[] = [
  "offline",
  "idle",
  "attentive",
  "acknowledging",
  "thinking",
  "planning",
  "working",
  "using-computer",
  "waiting",
  "needs-user",
  "blocked",
  "done",
  "error",
];

/** Accessible sentence for each mode (the canvas is never the only cue). */
export const PRESENCE_LABEL: Record<PresenceMode, string> = {
  offline: "Pegoles is offline",
  idle: "Pegoles",
  attentive: "Pegoles is listening",
  acknowledging: "Pegoles got it",
  thinking: "Pegoles is thinking",
  planning: "Pegoles is planning",
  working: "Pegoles is working",
  "using-computer": "Pegoles is using its computer",
  waiting: "Pegoles is waiting",
  "needs-user": "Pegoles needs you",
  blocked: "Pegoles can’t continue",
  done: "Pegoles is done",
  error: "Pegoles ran into a problem",
};

/** Modes that represent real work in progress (never paused just for losing focus). */
export const WORK_MODES: ReadonlySet<PresenceMode> = new Set(["thinking", "planning", "working", "using-computer"]);

/** One-shot modes: a single gesture on entry, then the pose holds still. */
export const GESTURE_MODES: ReadonlySet<PresenceMode> = new Set(["acknowledging", "done", "error"]);

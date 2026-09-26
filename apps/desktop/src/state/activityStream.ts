import type { TaskStatusWire } from "@pegoles/ui";
import { describeAction, describePast } from "../lib/events";
import { agentMessageOf, firstLine } from "../lib/execution";
import type { ActionRequestWire, AgentEvent, AgentTask } from "../lib/tauri";

/**
 * Activity as a dense log: grouped by day, then by task run (consecutive
 * entries of one task), with the computer's own lifecycle as its own
 * "Pegoles Computer" run. One short sentence per thing that happened;
 * action lifecycles collapse into one entry; boot handshakes and pointer
 * moves stay out of the way. Technical fields remain available under
 * Details, with typed text and file contents removed.
 */
export type EntryTone = "quiet" | "normal" | "attention" | "error" | "done";
/** What the row's glyph says about the outcome. */
export type EntryOutcome = "done" | "running" | "attention" | "error" | "neutral";

export interface ActivityEntry {
  readonly id: string;
  readonly at: string;
  readonly text: string;
  /** Where it happened, when that helps (a file's parent folder). */
  readonly context: string | null;
  readonly tone: EntryTone;
  readonly outcome: EntryOutcome;
  /** An action (as opposed to a task or computer lifecycle event). */
  readonly action: boolean;
  /** Still in progress (an action without its outcome yet). */
  readonly current: boolean;
  readonly taskId: string | null;
  readonly details: string;
}

export type GroupStatusTone = "active" | "attention" | "done" | "error" | "quiet";

export interface ActivityGroup {
  readonly key: string;
  readonly kind: "task" | "computer";
  readonly taskId: string | null;
  readonly title: string;
  /** Real task status, only on the task's newest run. */
  readonly status: TaskStatusWire | null;
  readonly statusLabel: string | null;
  readonly statusTone: GroupStatusTone | null;
  /** When the run started. */
  readonly at: string;
  /** Oldest first: a run reads like a story. */
  readonly entries: readonly ActivityEntry[];
}

export interface ActivityDay {
  readonly key: string;
  readonly label: string;
  /** Newest run first. */
  readonly groups: readonly ActivityGroup[];
  /** Every entry of the day, newest first. */
  readonly entries: readonly ActivityEntry[];
}

export const COMPUTER_GROUP_TITLE = "Pegoles Computer";

export const GROUP_STATUS: Record<TaskStatusWire, { label: string; tone: GroupStatusTone }> = {
  pending: { label: "Not started", tone: "quiet" },
  running: { label: "Working", tone: "active" },
  waiting_for_approval: { label: "Needs you", tone: "attention" },
  completed: { label: "Done", tone: "done" },
  failed: { label: "Couldn’t finish", tone: "error" },
  cancelled: { label: "Cancelled", tone: "quiet" },
};

const PRIVATE_KEYS = new Set(["text", "content", "keys_typed"]);

function redact(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, inner]) => [key, PRIVATE_KEYS.has(key) ? "(hidden)" : redact(inner)]));
  }
  return value;
}

function detailsOf(events: readonly AgentEvent[]): string {
  return events.map((event) => JSON.stringify(redact(event), null, 2)).join("\n");
}

function requestOf(event: AgentEvent): ActionRequestWire | null {
  const request = "request" in event ? event.request : null;
  return request && typeof request === "object" && typeof (request as ActionRequestWire).action_id === "string" &&
    typeof (request as ActionRequestWire).action?.type === "string" ? request as ActionRequestWire : null;
}

function hostOf(url: unknown): string | null {
  if (typeof url !== "string") return null;
  try { return new URL(url).host.replace(/^www\./, "") || null; } catch { return null; }
}

function pathParts(path: string): { name: string; parent: string | null } {
  const parts = path.split("/").filter(Boolean);
  return { name: parts[parts.length - 1] ?? path, parent: parts.length > 1 ? parts[parts.length - 2] : null };
}

const FILE_ACTIONS = new Set(["read_file", "write_file", "list_directory"]);

type Voice = "past" | "present";

/** A short label with the target when it is safe to show. Typed text never appears. */
export function actionLabel(action: ActionRequestWire["action"], voice: Voice = "past"): { text: string; context: string | null } {
  const base = voice === "past" ? describePast(action) : describeAction({ ...action, text: "" });
  if (action.type === "open_url") {
    const host = hostOf(action.url);
    if (!host) return { text: base, context: null };
    return { text: `${voice === "past" ? "Visited" : "Opening"} ${host}`, context: null };
  }
  if (typeof action.path === "string" && FILE_ACTIONS.has(action.type)) {
    const { name, parent } = pathParts(action.path);
    return { text: `${base.replace(/ a (file|folder)$/, "")} ${name}`, context: parent };
  }
  return { text: base, context: null };
}

const TERMINAL = new Set(["action_completed", "action_denied", "action_failed"]);

function atOf(event: AgentEvent): string | undefined {
  return "at" in event && typeof event.at === "string" ? event.at : undefined;
}

function actionEntry(events: readonly AgentEvent[], request: ActionRequestWire): ActivityEntry {
  const terminal = [...events].reverse().find((event) => TERMINAL.has(event.type));
  const past = actionLabel(request.action, "past");
  const present = actionLabel(request.action, "present");
  let label = past;
  let suffix = "";
  let tone: EntryTone = "normal";
  let outcome: EntryOutcome = "done";
  if (terminal?.type === "action_denied") {
    label = present; suffix = " — blocked by policy"; tone = "attention"; outcome = "attention";
  } else if (terminal?.type === "action_failed") {
    const error = "error" in terminal && typeof terminal.error === "string" ? terminal.error : "";
    label = present; suffix = ` — ${error.startsWith("interrupted") ? "interrupted" : "didn’t work"}`; tone = "error"; outcome = "error";
  } else if (terminal?.type === "action_completed") {
    const result = ("result" in terminal ? terminal.result : undefined) as { outcome?: string } | undefined;
    if (result?.outcome === "needs_approval") { label = present; suffix = " — needs approval"; tone = "attention"; outcome = "attention"; }
    else if (result?.outcome && result.outcome !== "executed") {
      label = present; suffix = ` — ${result.outcome.replaceAll("_", " ")}`; tone = "attention"; outcome = "attention";
    }
  } else {
    label = present; suffix = "…"; outcome = "running";
  }
  const last = events[events.length - 1];
  const at = atOf(last) ?? events.map(atOf).find(Boolean) ??
    ((terminal && "result" in terminal ? (terminal.result as { completed_at?: string }).completed_at : undefined)) ?? request.requested_at;
  return {
    id: `action-${request.action_id}`, at, text: `${label.text}${suffix}`, context: label.context, tone, outcome, action: true,
    current: !terminal, taskId: typeof request.task_id === "string" ? request.task_id : null, details: detailsOf(events),
  };
}

const TONE_OUTCOME: Record<EntryTone, EntryOutcome> = { quiet: "neutral", normal: "neutral", attention: "attention", error: "error", done: "done" };

/** One sentence for a non-action event, or null to leave it out. Task events live inside their task's run, so they don't repeat its title. */
function sentence(event: AgentEvent): { text: string; tone: EntryTone } | null {
  const e = event as Record<string, unknown>;
  switch (event.type) {
    case "task_created": return { text: "You handed this over", tone: "normal" };
    case "task_status_changed":
      switch (e.to) {
        case "running": return { text: "Started working", tone: "normal" };
        case "completed": return { text: "Finished", tone: "done" };
        case "failed": return { text: "Couldn’t finish", tone: "error" };
        case "waiting_for_approval": return { text: "Asked for your approval", tone: "attention" };
        case "cancelled": return { text: "Cancelled", tone: "quiet" };
        default: return null;
      }
    case "approval_requested": return { text: typeof e.reason === "string" ? `Asked for approval: ${e.reason}` : "Asked for your approval", tone: "attention" };
    case "agent_message": {
      // Pegoles' own words, first line only (the whole note is in its task and under Details).
      const message = agentMessageOf(event);
      if (!message) return null;
      const line = firstLine(message.text, 120);
      return { text: line, tone: message.kind === "summary" ? "done" : message.kind === "error" ? "error" : "normal" };
    }
    case "computer_created": return { text: "Set up its computer", tone: "normal" };
    case "computer_state_changed":
      switch (e.to) {
        case "running": return { text: "Started its computer", tone: "normal" };
        case "stopped": return e.from === "stopped" ? null : { text: "Stopped its computer", tone: "quiet" };
        case "paused": return { text: "Paused its computer", tone: "quiet" };
        case "error": return { text: "Its computer ran into a problem", tone: "error" };
        default: return null;
      }
    case "guest_runtime_ready":
      return { text: typeof e.ready_in_ms === "number" ? `Its computer is ready · ${(e.ready_in_ms / 1000).toFixed(1)} s` : "Its computer is ready", tone: "done" };
    case "guest_runtime_disconnected": return { text: "Lost the connection to its computer", tone: "error" };
    case "guest_runtime_incompatible": return { text: "Its computer needs an update", tone: "error" };
    case "guest_runtime_error": return { text: "Its computer reported a problem", tone: "error" };
    case "graphical_session_failed": return { text: "Its computer’s display couldn’t start", tone: "error" };
    case "control_ownership_changed":
      if (e.to === "user") return { text: "You took control of its computer", tone: "attention" };
      if (e.to === "agent") return { text: "Pegoles took control of its computer", tone: "normal" };
      return e.from === "user" ? { text: "You gave the computer back", tone: "normal" } : null;
    case "guest_runtime_waiting": case "guest_runtime_connected": case "guest_handshake_completed":
    case "display_attached": case "display_ready": case "display_detached": case "graphical_session_ready":
    case "frame_observed": case "input_capability_changed":
      return null;
    default:
      return { text: event.type.replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase()), tone: "quiet" };
  }
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function dayLabel(date: Date, now: Date): string {
  if (dayKey(date) === dayKey(now)) return "Today";
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (dayKey(date) === dayKey(yesterday)) return "Yesterday";
  return date.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

/** "17:42" in the viewer's clock. Empty when the timestamp is unreadable. */
export function clockTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function collectEntries(events: readonly AgentEvent[]): { entries: ActivityEntry[]; createdTitles: Map<string, string> } {
  const actions = new Map<string, { request: ActionRequestWire; events: AgentEvent[] }>();
  const entries: ActivityEntry[] = [];
  const createdTitles = new Map<string, string>();
  events.forEach((event, index) => {
    const request = requestOf(event);
    if (request) {
      if (request.action.type === "move_pointer") return;
      const group = actions.get(request.action_id);
      if (group) group.events.push(event);
      else actions.set(request.action_id, { request, events: [event] });
      return;
    }
    const e = event as Record<string, unknown>;
    if (event.type === "task_created" && typeof e.task_id === "string" && typeof e.title === "string") createdTitles.set(e.task_id, e.title);
    const said = sentence(event);
    const at = atOf(event) ?? "";
    if (!said || !at) return;
    entries.push({
      id: `${event.type}-${index}-${at}`, at, ...said, context: null, outcome: TONE_OUTCOME[said.tone], action: false, current: false,
      taskId: typeof e.task_id === "string" ? e.task_id : null, details: detailsOf([event]),
    });
  });
  for (const group of actions.values()) entries.push(actionEntry(group.events, group.request));
  return { entries, createdTitles };
}

/** Consecutive entries (oldest first) of one task — or of the computer — form a run. */
function runsOf(ascending: readonly ActivityEntry[]): ActivityEntry[][] {
  const runs: ActivityEntry[][] = [];
  for (const entry of ascending) {
    const run = runs[runs.length - 1];
    if (run && run[0].taskId === entry.taskId) run.push(entry);
    else runs.push([entry]);
  }
  return runs;
}

export function activityStream(events: readonly AgentEvent[], tasks: readonly AgentTask[], now: Date = new Date()): ActivityDay[] {
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const { entries, createdTitles } = collectEntries(events);
  const dated = entries
    .map((entry, order) => ({ entry, order, time: new Date(entry.at).getTime() }))
    .filter((item) => !Number.isNaN(item.time))
    .sort((a, b) => a.time - b.time || a.order - b.order);

  const days = new Map<string, { label: string; ascending: ActivityEntry[] }>();
  for (const { entry, time } of dated) {
    const date = new Date(time);
    const key = dayKey(date);
    const day = days.get(key) ?? { label: dayLabel(date, now), ascending: [] };
    day.ascending.push(entry);
    days.set(key, day);
  }

  const seenTasks = new Set<string>();
  const result = [...days.entries()].reverse().map(([key, day]) => {
    const groups = runsOf(day.ascending).reverse().map((run): ActivityGroup => {
      const taskId = run[0].taskId;
      const task = taskId ? byId.get(taskId) : undefined;
      const newest = taskId !== null && !seenTasks.has(taskId);
      if (taskId) seenTasks.add(taskId);
      const status = newest && task ? task.status : null;
      return {
        key: `${taskId ?? "computer"}-${run[0].id}`,
        kind: taskId ? "task" : "computer",
        taskId,
        title: taskId ? task?.title ?? createdTitles.get(taskId) ?? "Task" : COMPUTER_GROUP_TITLE,
        status,
        statusLabel: status ? GROUP_STATUS[status].label : null,
        statusTone: status ? GROUP_STATUS[status].tone : null,
        at: run[0].at,
        entries: run,
      };
    });
    return { key, label: day.label, groups, entries: [...day.ascending].reverse() };
  });
  return result;
}

export interface ActivitySummary {
  /** "today" when anything happened today, "all" when only earlier, "none" when nothing did. */
  readonly scope: "today" | "all" | "none";
  readonly tasks: number;
  readonly actions: number;
  /** Tasks waiting for your approval right now. */
  readonly needsYou: number;
}

export function activitySummary(days: readonly ActivityDay[], tasks: readonly AgentTask[], now: Date = new Date()): ActivitySummary {
  const needsYou = tasks.filter((task) => task.status === "waiting_for_approval").length;
  if (days.length === 0) return { scope: "none", tasks: 0, actions: 0, needsYou };
  const todayKey = dayKey(now);
  const today = days.filter((day) => day.key === todayKey);
  const scoped = today.length ? today : days;
  const entries = scoped.flatMap((day) => day.entries);
  return {
    scope: today.length ? "today" : "all",
    tasks: new Set(entries.map((entry) => entry.taskId).filter((id): id is string => id !== null)).size,
    actions: entries.filter((entry) => entry.action).length,
    needsYou,
  };
}

export type ActivityFilter = "all" | "needs-you" | "computer";

export function filterDays(days: readonly ActivityDay[], filter: ActivityFilter): readonly ActivityDay[] {
  if (filter === "all") return days;
  const waiting = new Set(days.flatMap((day) => day.groups)
    .filter((group) => group.status === "waiting_for_approval").map((group) => group.taskId));
  const keep = (group: ActivityGroup) => filter === "computer" ? group.kind === "computer" : waiting.has(group.taskId);
  return days
    .map((day) => {
      const groups = day.groups.filter(keep);
      return { ...day, groups, entries: groups.flatMap((group) => group.entries).reverse() };
    })
    .filter((day) => day.groups.length > 0);
}

/** Tasks that exist but have nothing in the log yet, newest first. */
export function tasksWithoutActivity(days: readonly ActivityDay[], tasks: readonly AgentTask[]): AgentTask[] {
  const active = new Set(days.flatMap((day) => day.entries).map((entry) => entry.taskId));
  return tasks.filter((task) => !active.has(task.id)).sort((a, b) => b.created_at.localeCompare(a.created_at));
}

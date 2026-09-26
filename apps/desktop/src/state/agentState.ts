import type { PresenceMode } from "../presence";
import { describeAction, describePast } from "../lib/events";
import { agentMessageOf, capabilityFor, firstLine, requestOf, type Capability } from "../lib/execution";
import type { ActionRequestWire, AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";

/**
 * VisualStateAdapter: real Core state in, what Pegoles looks like out.
 * Pure and synchronous. Nothing here invents progress: every mode maps to
 * a task status, an action event or a user action (see
 * docs/FRONTEND_EXPERIENCE.md §3). Hidden reasoning is never shown.
 */

export type ActionOutcome = "running" | "done" | "approval" | "blocked" | "failed" | "interrupted";

export interface ActionStep {
  readonly id: string;
  readonly label: string;
  /** Target of the action when it is safe to show (path, host). */
  readonly detail?: string;
  readonly capability?: Capability;
  readonly outcome: ActionOutcome;
  readonly at: string;
}

/** Why a pending task hasn't started: no model yet, ready to start, or Pegoles is busy with another task. */
export type StartState = "needs-model" | "ready" | "busy";

export interface TaskActivity {
  readonly mode: PresenceMode;
  /** One human line: what Pegoles is doing or needs. */
  readonly headline: string;
  readonly detail?: string;
  readonly capability?: Capability;
  readonly approvalReason?: string;
  /** Recent real actions, newest last (detail on demand). */
  readonly recent: readonly ActionStep[];
  /** Increments on every real event for this task (drives presence pulses). */
  readonly pulse: number;
  /** The task can still change: Pegoles' live turn is shown. */
  readonly live: boolean;
  /** Its computer is usable even though the task can't run. */
  readonly offerComputer?: boolean;
  /** Only for a task that hasn't started. */
  readonly start?: StartState;
  /** Why a finished run stopped, in Pegoles' words (first line). */
  readonly reason?: string;
  /** When a finished task reached its final state. */
  readonly at?: string;
}

const TERMINAL = new Set(["action_completed", "action_failed", "action_denied"]);

function hostOf(url: unknown): string | undefined {
  if (typeof url !== "string") return undefined;
  try { return new URL(url).host || undefined; } catch { return undefined; }
}

/** Safe, short target of an action. Typed text never appears here. */
function detailOf(action: ActionRequestWire["action"]): string | undefined {
  if (typeof action.path === "string") return action.path;
  if (action.type === "open_url") return hostOf(action.url);
  return undefined;
}

function outcomeOf(event: AgentEvent): ActionOutcome {
  if (event.type === "action_denied") return "blocked";
  if (event.type === "action_failed") {
    const error = "error" in event && typeof event.error === "string" ? event.error : "";
    return error.startsWith("interrupted") ? "interrupted" : "failed";
  }
  if (event.type === "action_completed") {
    const result = ("result" in event ? event.result : undefined) as { outcome?: string; success?: boolean } | undefined;
    if (result?.outcome === "needs_approval") return "approval";
    if (result?.outcome === "blocked") return "blocked";
    if (result?.outcome === "interrupted") return "interrupted";
    return result?.success === false || (result?.outcome && result.outcome !== "executed") ? "failed" : "done";
  }
  return "running";
}

function atOf(event: AgentEvent, request: ActionRequestWire): string {
  return "at" in event && typeof event.at === "string" ? event.at : request.requested_at;
}

/** One step per action id, in first-seen order, carrying its latest state. */
export function actionSteps(events: readonly AgentEvent[]): ActionStep[] {
  const steps = new Map<string, ActionStep>();
  for (const event of events) {
    const request = requestOf(event);
    if (!request) continue;
    const previous = steps.get(request.action_id);
    const outcome = outcomeOf(event);
    // A late non-terminal echo never reopens a finished action.
    if (previous && previous.outcome !== "running" && !TERMINAL.has(event.type)) continue;
    steps.set(request.action_id, {
      id: request.action_id,
      label: outcome === "running" ? describeAction({ ...request.action, text: "" }) : describePast(request.action),
      detail: detailOf(request.action),
      capability: capabilityFor(request.action.type),
      outcome,
      at: previous?.at ?? atOf(event, request),
    });
  }
  return [...steps.values()];
}

/** What a capability looks like as a headline while it is in use. */
const CAPABILITY_HEADLINE: Record<Capability, string> = {
  Computer: "Using its computer",
  Files: "Working with files",
  Shell: "Running a command",
  Web: "Opening a website",
};

export interface TaskActivityInputs {
  readonly task: AgentTask;
  readonly events: readonly AgentEvent[];
  readonly status: StatusPayload | null;
  readonly connected: boolean;
  /** This task was just handed over and Core has not moved it yet. */
  readonly acknowledging?: boolean;
  /** A start of this task is in flight (Core hasn't answered yet). */
  readonly starting?: boolean;
}

export function modelConnected(status: StatusPayload | null): boolean {
  return status?.model === "configured";
}

/** The latest note of a kind Pegoles wrote for this task, as one line. */
function latestNote(events: readonly AgentEvent[], kind: "progress" | "error"): string | undefined {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const message = agentMessageOf(events[i]);
    if (message?.kind === kind) return firstLine(message.text);
  }
  return undefined;
}

function approvalReason(events: readonly AgentEvent[]): string | undefined {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    if (event.type === "approval_requested" && typeof event.reason === "string") return event.reason;
    if (event.type === "action_evaluated") {
      const verdict = event.verdict as { decision?: string; reason?: string } | undefined;
      if (verdict?.decision === "require_approval" && typeof verdict.reason === "string") return verdict.reason;
    }
  }
  return undefined;
}

export function taskActivity({ task, events, status, connected, acknowledging = false, starting = false }: TaskActivityInputs): TaskActivity {
  const recent = actionSteps(events).slice(-6);
  const pulse = events.length;
  const base = { recent, pulse };
  if (!connected) return { ...base, mode: "offline", headline: "Waiting for Pegoles Core", live: false };
  const last = recent[recent.length - 1];
  if (task.status === "completed") return { ...base, mode: "done", headline: "Done", live: false, at: task.updated_at };
  if (task.status === "failed") {
    const stopped = [...recent].reverse().find((step) => step.outcome !== "done");
    return { ...base, mode: "error", headline: "Couldn’t finish this", detail: stopped?.label, reason: latestNote(events, "error"), live: false, at: task.updated_at };
  }
  if (task.status === "cancelled") return { ...base, mode: "idle", headline: "Cancelled", reason: latestNote(events, "error"), live: false, at: task.updated_at };

  const needsApproval = task.status === "waiting_for_approval" || last?.outcome === "approval" ||
    (events.length > 0 && events[events.length - 1].type === "approval_requested");
  if (needsApproval) {
    return { ...base, mode: "needs-user", headline: "Needs your approval", approvalReason: approvalReason(events), live: true };
  }
  if (last?.outcome === "blocked") {
    return { ...base, mode: "blocked", headline: "Stopped by a safety rule", detail: last.label, live: true };
  }
  if (last?.outcome === "failed") {
    return { ...base, mode: "error", headline: "An action didn’t work", detail: last.label, live: true };
  }
  if (status?.control_owner === "user" && (task.status === "running" || last?.outcome === "running")) {
    return { ...base, mode: "waiting", headline: "Waiting while you use its computer", live: true };
  }
  // "Started" is only believed while Core corroborates that input is busy.
  if (last?.outcome === "running" && status?.agent_busy) {
    const capability = last.capability;
    return {
      ...base, capability, live: true,
      mode: capability === "Computer" ? "using-computer" : "working",
      headline: capability ? CAPABILITY_HEADLINE[capability] : last.label,
      detail: capability ? last.detail ?? last.label : last.detail,
    };
  }
  if (acknowledging) return { ...base, mode: "acknowledging", headline: "Got it", live: true };
  // Between actions, the line says what Pegoles last wrote about its work.
  if (task.status === "running") return { ...base, mode: "thinking", headline: "Working on it", detail: latestNote(events, "progress"), live: true };
  // Not started. A start in flight, or a run claimed but not yet reported as running, is starting.
  if (starting || status?.active_task === task.id) return { ...base, mode: "thinking", headline: "Starting", live: true };
  if (!modelConnected(status)) {
    return {
      ...base, mode: "blocked", headline: "Can’t start yet", live: true, offerComputer: true, start: "needs-model",
      detail: "Pegoles needs a model to work on tasks. Add an Anthropic API key in Settings, then start it. The task is kept while the app is open, and its computer still works.",
    };
  }
  if (status?.active_task) {
    return { ...base, mode: "idle", headline: "Not started", detail: "Pegoles is working on another task", live: false, start: "busy" };
  }
  return { ...base, mode: "idle", headline: "Not started", live: false, start: "ready" };
}

export interface GlobalPresenceInputs {
  readonly connected: boolean;
  readonly tasks: readonly AgentTask[];
  readonly status: StatusPayload | null;
  readonly acknowledging: boolean;
  readonly attentive: boolean;
  /** The computer is being set up or booting. */
  readonly computerTransitioning: boolean;
}

/** The one Pegoles the whole app shows when no single task is in focus. */
export function globalPresence(input: GlobalPresenceInputs): PresenceMode {
  const { connected, tasks, status } = input;
  if (!connected) return "offline";
  if (input.acknowledging) return "acknowledging";
  if (tasks.some((task) => task.status === "waiting_for_approval")) return "needs-user";
  if (status?.control_owner === "user") return "waiting";
  if (status?.agent_busy || status?.viewport_state === "agent_active") return "using-computer";
  if (tasks.some((task) => task.status === "running")) return "thinking";
  if (input.computerTransitioning) return "working";
  if (input.attentive) return "attentive";
  return "idle";
}

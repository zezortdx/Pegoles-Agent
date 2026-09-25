import { requestOf, type Capability } from "../lib/execution";
import type { AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";
import { actionSteps, type ActionOutcome } from "./agentState";

/**
 * Thought Ribbon model: real Core state in, a small status visualization
 * out. One strand per real action id (the four most recent), growing out
 * of a trunk that reflects the task itself. Pure and synchronous.
 *
 * Honesty rules: there is no plan signal in Core, so strands only ever
 * come from `action_*` events: nothing fans out ahead of time. "Running"
 * is only believed while Core corroborates that input is busy (same rule
 * as taskActivity). Hidden reasoning is never shown.
 */

export type RibbonTrunk = "dormant" | "thinking" | "working" | "waiting" | "done";
export type RibbonPhase = "sprouting" | "approval" | "running" | "done" | "failed" | "blocked" | "interrupted";

export interface RibbonStrand {
  readonly id: string;
  readonly capability?: Capability;
  /** Short human line for the action (never typed text). */
  readonly label: string;
  readonly phase: RibbonPhase;
  readonly at: string;
}

export interface RibbonModel {
  readonly trunk: RibbonTrunk;
  readonly strands: readonly RibbonStrand[];
  /** The one strand that is in flight right now, if any. */
  readonly activeId?: string;
}

export interface RibbonInput {
  readonly task: AgentTask;
  readonly events: readonly AgentEvent[];
  readonly status: StatusPayload | null;
  readonly connected: boolean;
}

/** Strands shown at most: the ribbon is a glance, not a history. */
export const RIBBON_MAX_STRANDS = 4;

const IN_FLIGHT: ReadonlySet<RibbonPhase> = new Set(["sprouting", "approval", "running"]);

export function isInFlight(phase: RibbonPhase): boolean {
  return IN_FLIGHT.has(phase);
}

function belongsTo(event: AgentEvent, taskId: string): boolean {
  const request = requestOf(event);
  if (request) return request.task_id === taskId;
  return "task_id" in event && event.task_id === taskId;
}

interface Signals {
  readonly started: ReadonlySet<string>;
  readonly needsApproval: ReadonlySet<string>;
}

function signalsOf(events: readonly AgentEvent[]): Signals {
  const started = new Set<string>();
  const needsApproval = new Set<string>();
  for (const event of events) {
    const request = requestOf(event);
    if (!request) continue;
    if (event.type === "action_started") started.add(request.action_id);
    if (event.type === "action_evaluated") {
      const verdict = event.verdict as { decision?: string } | undefined;
      if (verdict?.decision === "require_approval") needsApproval.add(request.action_id);
    }
  }
  return { started, needsApproval };
}

function phaseOf(outcome: ActionOutcome, id: string, signals: Signals, busy: boolean): RibbonPhase {
  switch (outcome) {
    case "done": return "done";
    case "failed": return "failed";
    case "blocked": return "blocked";
    case "interrupted": return "interrupted";
    case "approval": return "approval";
    case "running":
      if (signals.needsApproval.has(id) && !signals.started.has(id)) return "approval";
      return signals.started.has(id) && busy ? "running" : "sprouting";
  }
}

const TERMINAL_TASK = new Set(["completed", "failed", "cancelled"]);

export function ribbonModel({ task, events, status, connected }: RibbonInput): RibbonModel {
  const own = events.filter((event) => belongsTo(event, task.id));
  const signals = signalsOf(own);
  const busy = connected && !!status?.agent_busy && status.control_owner !== "user";
  const finished = TERMINAL_TASK.has(task.status);
  const all = actionSteps(own).map((step): RibbonStrand => {
    const phase = phaseOf(step.outcome, step.id, signals, busy);
    // A finished task has nothing in flight: an action it never closed was cut short.
    const settled = finished && isInFlight(phase) ? "interrupted" : phase;
    return { id: step.id, capability: step.capability, label: step.label, phase: settled, at: step.at };
  });
  let strands = all.slice(-RIBBON_MAX_STRANDS);
  // The last request still waiting for a person is the approval, even before Core evaluates it.
  const last = strands[strands.length - 1];
  if (task.status === "waiting_for_approval" && last?.phase === "sprouting") {
    strands = [...strands.slice(0, -1), { ...last, phase: "approval" }];
  }
  const active = connected && !finished ? [...strands].reverse().find((strand) => isInFlight(strand.phase)) : undefined;
  return { trunk: trunkOf(task, status, connected, active), strands, activeId: active?.id };
}

function trunkOf(task: AgentTask, status: StatusPayload | null, connected: boolean, active: RibbonStrand | undefined): RibbonTrunk {
  if (!connected) return "dormant";
  if (task.status === "completed") return "done";
  if (TERMINAL_TASK.has(task.status)) return "dormant";
  if (task.status === "waiting_for_approval" || active?.phase === "approval") return "waiting";
  if (status?.control_owner === "user" && (task.status === "running" || active)) return "waiting";
  if (active?.phase === "running") return "working";
  if (active || task.status === "running") return "thinking";
  return "dormant";
}

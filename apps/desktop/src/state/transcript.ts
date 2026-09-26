import { actionSteps, type ActionStep } from "./agentState";
import { agentMessageOf, fileArtifacts, requestOf, type AgentMessage } from "../lib/execution";
import type { AgentEvent, AgentMessageKind, AgentTask } from "../lib/tauri";

/**
 * A task is one mixed timeline of work. Each item has its own shape:
 * requests read as requests, files as files, approvals as approvals, and
 * what Pegoles wrote reads as its words (plain text, never markup).
 * Items come only from the task record and its real events.
 */

/** A finished action as the transcript shows it. */
export interface TranscriptStep extends ActionStep {
  /** The wire action type ("click", "shell", …). */
  readonly verb: string;
  /** How long Core says the action took; absent when Core did not report it. */
  readonly durationMs?: number;
  /** Changes something outside Pegoles' own looking around: never folded away. */
  readonly consequential: boolean;
}

export type OutcomeStatus = "completed" | "failed" | "cancelled";

export type TranscriptItem =
  | { readonly kind: "request"; readonly id: string; readonly text: string; readonly at: string }
  | { readonly kind: "actions"; readonly id: string; readonly steps: readonly TranscriptStep[]; readonly at: string }
  | { readonly kind: "file"; readonly id: string; readonly path: string; readonly at: string }
  | { readonly kind: "approval"; readonly id: string; readonly reason?: string; readonly open: boolean; readonly at: string }
  /** Pegoles' own words: a note between actions, its final summary, or why the run stopped. Untrusted text. */
  | { readonly kind: "message"; readonly id: string; readonly variant: AgentMessageKind; readonly text: string; readonly at: string }
  /** The task reached a final state; counts are finished actions and written files. */
  | { readonly kind: "outcome"; readonly id: string; readonly status: OutcomeStatus; readonly actions: number; readonly files: number; readonly at: string };

/** Verbs whose effects outlive the action: always visible, even in a folded group. */
const CONSEQUENTIAL = new Set([
  "write_file", "delete_file", "delete", "move_file", "rename_file", "submit", "download", "shell", "run_command",
]);

export function isConsequential(verb: string): boolean {
  return CONSEQUENTIAL.has(verb);
}

function eventAt(event: AgentEvent): string | undefined {
  if ("at" in event && typeof event.at === "string") return event.at;
  return requestOf(event)?.requested_at;
}

interface StepFacts { readonly verb: string; readonly durationMs?: number }

/** Verb and reported duration per action id, validated at the boundary. */
function stepFacts(events: readonly AgentEvent[]): Map<string, StepFacts> {
  const facts = new Map<string, StepFacts>();
  for (const event of events) {
    const request = requestOf(event);
    if (!request) continue;
    const previous = facts.get(request.action_id);
    let durationMs = previous?.durationMs;
    if (event.type === "action_completed" && "result" in event) {
      const reported = (event.result as { duration_ms?: unknown } | undefined)?.duration_ms;
      if (typeof reported === "number" && Number.isFinite(reported) && reported >= 0) durationMs = reported;
    }
    facts.set(request.action_id, { verb: request.action.type, durationMs });
  }
  return facts;
}

function toStep(step: ActionStep, facts: ReadonlyMap<string, StepFacts>): TranscriptStep {
  const fact = facts.get(step.id);
  const verb = fact?.verb ?? "";
  return { ...step, verb, durationMs: fact?.durationMs, consequential: isConsequential(verb) };
}

/** Milliseconds since the epoch; unreadable timestamps sort last. */
function timeOf(at: string): number {
  const time = Date.parse(at);
  return Number.isFinite(time) ? time : Number.MAX_SAFE_INTEGER;
}

/**
 * Finished actions, grouped into runs between other kinds of items. A note
 * Pegoles wrote (`breaks`, ascending times) ends the run before it, so the
 * story reads note → what it did → next note.
 */
function actionRuns(steps: readonly TranscriptStep[], fileIds: ReadonlySet<string>, breaks: readonly number[]): TranscriptItem[] {
  const runs: TranscriptItem[] = [];
  let current: TranscriptStep[] = [];
  let next = 0;
  const flush = () => {
    if (current.length) runs.push({ kind: "actions", id: `actions-${current[0].id}`, steps: current, at: current[0].at });
    current = [];
  };
  for (const step of steps) {
    const at = timeOf(step.at);
    while (next < breaks.length && breaks[next] <= at) { flush(); next += 1; }
    // A written file is shown as a file, not also as an action row.
    if (fileIds.has(step.id)) { flush(); continue; }
    // An approval stop is shown as an approval item.
    if (step.outcome === "approval") { flush(); continue; }
    current.push(step);
  }
  flush();
  return runs;
}

export interface TranscriptInputs {
  readonly task: AgentTask;
  readonly events: readonly AgentEvent[];
}

const FINAL: ReadonlySet<string> = new Set<OutcomeStatus>(["completed", "failed", "cancelled"]);

/** At the same instant, what Pegoles wrote comes before what it then did. */
function chronological(a: TranscriptItem, b: TranscriptItem): number {
  return timeOf(a.at) - timeOf(b.at) || Number(b.kind === "message") - Number(a.kind === "message");
}

function messagesOf(events: readonly AgentEvent[], taskId: string): AgentMessage[] {
  return events.map(agentMessageOf).filter((message): message is AgentMessage => message?.taskId === taskId);
}

/** Everything that already happened. Pegoles' own turn is rendered after it. */
export function buildTranscript({ task, events }: TranscriptInputs): TranscriptItem[] {
  const items: TranscriptItem[] = [{ kind: "request", id: `request-${task.id}`, text: task.title, at: task.created_at }];

  const files = fileArtifacts([...events]);
  const fileIds = new Set(files.map((file) => file.id));
  const facts = stepFacts(events);
  const steps = actionSteps(events).filter((step) => step.outcome !== "running").map((step) => toStep(step, facts));
  const messages = messagesOf(events, task.id);
  const breaks = messages.map((message) => timeOf(message.at)).sort((a, b) => a - b);
  const timeline: TranscriptItem[] = [
    ...actionRuns(steps, fileIds, breaks),
    ...files.map((file): TranscriptItem => {
      const step = steps.find((candidate) => candidate.id === file.id);
      return { kind: "file", id: `file-${file.id}`, path: file.path, at: step?.at ?? task.updated_at };
    }),
  ];

  const approvals = events.filter((event) => event.type === "approval_requested");
  approvals.forEach((event, index) => {
    const reason = "reason" in event && typeof event.reason === "string" ? event.reason : undefined;
    const open = index === approvals.length - 1 && task.status === "waiting_for_approval";
    timeline.push({ kind: "approval", id: `approval-${index}-${eventAt(event) ?? ""}`, reason, open, at: eventAt(event) ?? task.updated_at });
  });
  if (!approvals.length && task.status === "waiting_for_approval") {
    timeline.push({ kind: "approval", id: "approval-status", open: true, at: task.updated_at });
  }

  messages.forEach((message, index) => {
    timeline.push({ kind: "message", id: `message-${index}-${message.at}`, variant: message.kind, text: message.text, at: message.at });
  });

  timeline.sort(chronological);
  items.push(...timeline);

  if (FINAL.has(task.status)) {
    items.push({ kind: "outcome", id: `outcome-${task.id}`, status: task.status as OutcomeStatus, actions: steps.length, files: files.length, at: task.updated_at });
  }
  return items;
}

import type { PresenceMode } from "../presence";
import type { AgentEvent, TeamStatus, TeamStepId } from "../lib/tauri";

/**
 * Experimental team mode, as the UI sees it: derived only from Core's
 * `team_progress` events for one task. Nothing here invents progress, and
 * the summaries are untrusted model text: the UI renders them as plain
 * text and never as markup.
 */

export const TEAM_STEPS: readonly TeamStepId[] = ["captain_plan", "research", "execute", "critique", "synthesis"];
export type TeamRole = "captain" | "researcher" | "executor" | "critic";
export const TEAM_ROLES: readonly TeamRole[] = ["captain", "researcher", "executor", "critic"];
export type TeamStepState = TeamStatus | "pending";

/** The role that owns each step. The captain owns two. */
export const STEP_ROLE: Record<TeamStepId, TeamRole> = {
  captain_plan: "captain", research: "researcher", execute: "executor", critique: "critic", synthesis: "captain",
};

/** Longest summary kept (characters): matches Core's bound. */
export const MAX_SUMMARY_CHARS = 400;

export interface TeamStepView {
  readonly step: TeamStepId;
  readonly role: TeamRole;
  readonly state: TeamStepState;
  /** Plain text, bounded. */
  readonly summary: string;
}

export interface TeamRun {
  readonly steps: readonly TeamStepView[];
  /** Each role's state: the captain shows its latest of plan and synthesis. */
  readonly roles: Readonly<Record<TeamRole, TeamStepState>>;
  /** The step in progress, if any. */
  readonly active: TeamStepId | null;
  /** Every step has settled (done, failed or skipped). */
  readonly settled: boolean;
  readonly failed: boolean;
}

interface Cell { state: TeamStepState; summary: string }

const STATUSES: ReadonlySet<string> = new Set<TeamStatus>(["working", "done", "failed", "skipped"]);
const STEP_SET: ReadonlySet<string> = new Set(TEAM_STEPS);

function bounded(text: string): string {
  const chars = [...text.trim()];
  return chars.length > MAX_SUMMARY_CHARS ? chars.slice(0, MAX_SUMMARY_CHARS).join("").trimEnd() : chars.join("");
}

/** A `team_progress` event validated at the boundary; null for anything malformed. */
export function teamEventOf(event: AgentEvent): { step: TeamStepId; status: TeamStatus; summary: string } | null {
  if (event.type !== "team_progress") return null;
  const { step, status, summary } = event as Record<string, unknown>;
  if (typeof step !== "string" || !STEP_SET.has(step)) return null;
  if (typeof status !== "string" || !STATUSES.has(status)) return null;
  return { step: step as TeamStepId, status: status as TeamStatus, summary: typeof summary === "string" ? bounded(summary) : "" };
}

/**
 * The team's run from a task's events, in arrival order. A step that starts
 * working again clears the steps after it: they are stale (a person used
 * the computer, so the executor looks again). Null when the task has no
 * team events, i.e. it ran with a single model.
 */
export function teamRunOf(events: readonly AgentEvent[]): TeamRun | null {
  const pending = (): Cell => ({ state: "pending", summary: "" });
  const cells: Record<TeamStepId, Cell> = {
    captain_plan: pending(), research: pending(), execute: pending(), critique: pending(), synthesis: pending(),
  };
  let seen = false;
  for (const event of events) {
    const next = teamEventOf(event);
    if (!next) continue;
    seen = true;
    if (next.status === "working") {
      for (const later of TEAM_STEPS.slice(TEAM_STEPS.indexOf(next.step) + 1)) cells[later] = pending();
    }
    cells[next.step] = { state: next.status, summary: next.summary || (next.status === "working" ? "" : cells[next.step].summary) };
  }
  if (!seen) return null;
  const steps = TEAM_STEPS.map((step): TeamStepView => ({ step, role: STEP_ROLE[step], ...cells[step] }));
  const roles: Record<TeamRole, TeamStepState> = {
    captain: cells.synthesis.state !== "pending" ? cells.synthesis.state : cells.captain_plan.state,
    researcher: cells.research.state,
    executor: cells.execute.state,
    critic: cells.critique.state,
  };
  return {
    steps, roles,
    active: steps.find((step) => step.state === "working")?.step ?? null,
    settled: steps.every((step) => step.state !== "pending" && step.state !== "working"),
    failed: steps.some((step) => step.state === "failed"),
  };
}

/** What each role's avatar shows for its real state. */
export function modeFor(state: TeamStepState): PresenceMode {
  switch (state) {
    case "working": return "working";
    case "done": return "done";
    case "failed": return "error";
    default: return "idle";
  }
}

import type { Capability } from "../lib/execution";
import type { TranscriptStep } from "./transcript";

/**
 * How a run of finished actions reads when folded: one line that says
 * what kind of work it was, how much, and how long Core measured it took.
 */
const RUN_LABEL: Record<Capability, string> = {
  Computer: "Used its computer",
  Files: "Worked with files",
  Shell: "Ran commands",
  Web: "Browsed the web",
};

export interface RunSummary {
  readonly label: string;
  readonly count: number;
  /** Sum of Core-reported durations; absent unless every step reported one. */
  readonly durationMs?: number;
  /** The one capability every step shares, if any. */
  readonly capability?: Capability;
}

/** Distinct things it did, in order, as one short sentence: "Looked at the screen, clicked and typed text". */
export function runSentence(steps: readonly TranscriptStep[], limit = 3): string {
  const said = [...new Set(steps.map((step) => step.label))];
  const shown = said.slice(0, limit).map((label, index) => (index === 0 ? label : label.charAt(0).toLowerCase() + label.slice(1)));
  if (said.length > limit) return `${shown.join(", ")} and more`;
  if (shown.length < 2) return shown[0] ?? "";
  return `${shown.slice(0, -1).join(", ")} and ${shown[shown.length - 1]}`;
}

export function runSummary(steps: readonly TranscriptStep[]): RunSummary {
  const kinds = new Set(steps.map((step) => step.capability));
  const only = kinds.size === 1 ? [...kinds][0] : undefined;
  const measured = steps.every((step) => typeof step.durationMs === "number");
  return {
    label: runSentence(steps) || (only ? RUN_LABEL[only] : "Worked on it"),
    count: steps.length,
    durationMs: measured && steps.length ? steps.reduce((sum, step) => sum + (step.durationMs ?? 0), 0) : undefined,
    capability: only,
  };
}

/** A single action reads better as itself than as a summary of one. */
export const FOLD_FROM = 2;

/** Steps shown under a folded run: everything whose effect outlives it, and anything that went wrong. */
export function pinnedSteps(steps: readonly TranscriptStep[]): TranscriptStep[] {
  return steps.filter((step) => step.consequential || (step.outcome !== "done" && step.outcome !== "running"));
}

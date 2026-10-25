import type { CopyKey } from "../i18n/catalog";
import type { AvatarProfile } from "../preferences/types";
import type { TeamRole, TeamStepState } from "../state/teamRun";
import type { TeamStepId } from "../lib/tauri";

/** Product copy for the four roles. Static keys only: model text never becomes a key. */
export const ROLE_NAME: Record<TeamRole, CopyKey> = {
  captain: "Captain", researcher: "Researcher", executor: "Executor", critic: "Critic",
};

export const STEP_TITLE: Record<TeamStepId, CopyKey> = {
  captain_plan: "Captain: plan",
  research: "Researcher: look at the screen",
  execute: "Executor: do the work",
  critique: "Critic: check the result",
  synthesis: "Captain: final answer",
};

export const STATE_WORD: Record<TeamStepState, CopyKey> = {
  pending: "Waiting", working: "Working", done: "Done", failed: "Failed", skipped: "Skipped",
};

/**
 * Four distinct body shapes in the silver/neutral palette, so the roles
 * differ by silhouette (and by their written name), never by hue alone.
 * The profile carries no saved name: the role's own name labels it.
 */
export const ROLE_PROFILE: Record<TeamRole, Omit<AvatarProfile, "name">> = {
  captain: { shape: "hexagon", color: "pearl", expression: "attentive" },
  researcher: { shape: "droplet", color: "silver", expression: "curious" },
  executor: { shape: "squircle", color: "graphite", expression: "neutral" },
  critic: { shape: "triangle", color: "silver", expression: "suspicious" },
};

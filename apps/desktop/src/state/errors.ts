/**
 * The product talks to people; diagnostics talk to engineers. Every raw
 * Core/IPC error becomes one calm sentence, an optional hint and the
 * verbatim detail (shown only under "Details").
 */
export type ErrorScope = "task" | "run" | "computer" | "general";

export interface HumanError {
  readonly scope: ErrorScope;
  readonly title: string;
  readonly hint?: string;
  /** Verbatim backend text, for the Details disclosure only. */
  readonly detail: string;
  /** "Try again" makes sense for this failure. */
  readonly retryable: boolean;
}

interface Rule {
  readonly test: RegExp;
  readonly scope?: ErrorScope;
  readonly title: string;
  readonly hint?: string;
  readonly retryable?: boolean;
  /** Also names why a run stopped (the planner's words in the task's failure note). */
  readonly stop?: boolean;
}

const COMPUTER_TITLE = "Computer couldn’t start.";
const START_TITLE = "Pegoles couldn’t start its computer.";

/** "Mac" or "PC", for sentences about the machine people are on. */
const HOST = typeof navigator !== "undefined" && /windows/i.test(navigator.userAgent) ? "PC" : "Mac";

/**
 * Who plans. Before the computer rules: an integrity failure mentions
 * checksums, and it is the local model's, not the computer image's.
 */
const INTELLIGENCE_RULES: readonly Rule[] = [
  { test: /failed its integrity check/i, scope: "run", title: "Pegoles Local needs to be set up again.",
    hint: "Its files didn’t pass the integrity check. Remove it in Settings, then set it up again.", retryable: false, stop: true },
  { test: /Set up Pegoles Local/i, scope: "run", title: "Pegoles Local isn’t set up yet.",
    hint: `Set it up in Settings (free, runs on this ${HOST}), or connect a cloud model.`, retryable: false },
  { test: /Connect a model/i, scope: "run", title: "Cloud mode needs an Anthropic key.",
    hint: `Add one in Settings, or switch to Pegoles Local (free, runs on this ${HOST}).`, retryable: false },
  { test: /local model runtime is not installed/i, scope: "run", title: `Pegoles Local can’t run on this ${HOST} yet.`,
    hint: "Its runtime isn’t set up. You can connect a cloud model in Settings instead.", retryable: false, stop: true },
  { test: /not enough memory to run the local model/i, scope: "run", title: "Not enough free memory for Pegoles Local.",
    hint: "Quit some apps to free memory, then try again.", stop: true },
  { test: /local model runtime stopped unexpectedly/i, scope: "run", title: "Pegoles Local stopped unexpectedly.",
    hint: "Try again. If it keeps happening, restart Pegoles.", stop: true },
  { test: /local model timed out/i, scope: "run", title: "Pegoles Local took too long to answer.", hint: "Try again.", stop: true },
  { test: /local model could not load/i, scope: "run", title: "Pegoles Local couldn’t load its model.",
    hint: "If it keeps happening, remove it in Settings and set it up again.", stop: true },
  { test: /kept repeating the same action/i, scope: "run", title: "Stopped: it kept repeating the same action.",
    hint: "Nothing changed on its screen, so Pegoles stopped instead of looping. Try describing the task differently.", stop: true },
  { test: /did not produce a valid action/i, scope: "run", title: "Stopped: Pegoles Local couldn’t decide on a next step.",
    hint: "Its answers couldn’t be turned into an action. Try describing the task differently.", stop: true },
  { test: /rejected the API key/i, scope: "run", title: "Anthropic didn’t accept the API key.",
    hint: "Check the key in Settings, or switch to Pegoles Local.", retryable: false, stop: true },
];

/**
 * The computer failing in ways people can do something about. Before the
 * generic computer rules: Windows reports these as HRESULTs.
 */
const HOST_RULES: readonly Rule[] = [
  { test: /0x80370102|HCS_E_HYPERV_NOT_INSTALLED|virtualization (is )?(disabled|not enabled|turned off)|VirtualMachinePlatform/i, scope: "computer",
    title: "Virtualization needs to be turned on.",
    hint: "Pegoles uses hardware virtualization to give the AI its own isolated computer. Restart Pegoles to check this PC and turn it on.", retryable: false },
  { test: /0x8037011B|HCS_E_ACCESS_DENIED|pegoles-vm-broker|broker (is )?(not running|unavailable|not installed)/i, scope: "computer",
    title: START_TITLE, hint: "Part of Pegoles that runs its computer isn’t available. Reinstalling Pegoles fixes it." },
  { test: /handshake|guest (runtime )?(did not answer|stopped responding|not responding|timed? ?out)|guest_timeout/i, scope: "computer",
    title: START_TITLE, hint: "Try restarting Pegoles. If the problem continues, open Technical details." },
  { test: /\bOOM\b|out of memory|ERROR_NOT_ENOUGH_MEMORY|0x8007000E/i, scope: "computer",
    title: "Pegoles ran out of available memory.", hint: "Close some applications and try again." },
];

const RULES: readonly Rule[] = [
  ...INTELLIGENCE_RULES,
  ...HOST_RULES,
  { test: /vm-host binary not found|PEGOLES_VM_HOST/i, scope: "computer", title: COMPUTER_TITLE,
    hint: "The helper that runs Pegoles’ computer isn’t installed on this Mac." },
  { test: /not_entitled|entitlement|com\.apple\.security\.virtualization/i, scope: "computer", title: COMPUTER_TITLE,
    hint: "This build of Pegoles isn’t allowed to use virtualization on this Mac." },
  { test: /no space|ENOSPC|disk full/i, scope: "computer", title: "Not enough disk space.",
    hint: "Pegoles’ computer needs a few gigabytes free." },
  { test: /computer image|image missing|checksum|sha256/i, scope: "computer", title: "Pegoles’ computer image isn’t ready.",
    hint: "The Pegoles computer image isn’t installed on this Mac, or it’s incomplete." },
  { test: /is already running/i, scope: "run", title: "Pegoles is working on another task.",
    hint: "It works on one task at a time. Start this one when that one ends." },
  { test: /not pending/i, scope: "run", title: "This task has already started." },
  { test: /before (resetting|removing) the computer/i, title: "Stop the running task first.",
    hint: "Pegoles’ computer can’t be reset or removed while it works on a task." },
  { test: /Live updates unavailable/i, scope: "general", title: "Live updates paused.",
    hint: "Pegoles will keep checking every few seconds." },
];

const SCOPE_DEFAULT: Record<ErrorScope, { title: string; hint?: string }> = {
  task: { title: "Couldn’t hand this to Pegoles.", hint: "Your text is still here." },
  run: { title: "Pegoles couldn’t start this task.", hint: "The task is kept. Try starting it again." },
  computer: { title: "Pegoles’ computer needs attention." },
  general: { title: "Something went wrong." },
};

/** Tauri rejects with strings; JS failures arrive as Error. */
export function errorText(raw: unknown): string {
  if (raw instanceof Error) return raw.message;
  if (typeof raw === "string") return raw;
  try { return JSON.stringify(raw); } catch { return String(raw); }
}

/**
 * A task run's failure stays with its task (shown where the task is), even
 * when the words come from a computer rule.
 */
function applies(rule: Rule, scope: ErrorScope): boolean {
  return !rule.scope || rule.scope === scope || scope === "general" || scope === "run";
}

export function humanizeError(raw: unknown, scope: ErrorScope): HumanError {
  const detail = errorText(raw).trim() || "No details were reported.";
  const rule = RULES.find((candidate) => candidate.test.test(detail) && applies(candidate, scope));
  if (rule) {
    const kept = scope === "run" ? scope : rule.scope ?? scope;
    return { scope: kept, title: rule.title, hint: rule.hint, detail, retryable: rule.retryable ?? true };
  }
  const fallback = SCOPE_DEFAULT[scope];
  return { scope, title: fallback.title, hint: fallback.hint, detail, retryable: true };
}

/**
 * Why a run stopped, calmly: the planner's known failures ("The model
 * could not continue: model unavailable: …") become one sentence; any
 * other note (such as "Stopped by the user.") stays in Pegoles' words.
 */
export function stopReason(note: string): string {
  return INTELLIGENCE_RULES.find((rule) => rule.stop && rule.test.test(note))?.title ?? note;
}

/**
 * The product talks to people; diagnostics talk to engineers. Every raw
 * Core/IPC error becomes one calm sentence, an optional hint and the
 * verbatim detail (shown only under "Details").
 */
export type ErrorScope = "task" | "computer" | "general";

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
}

const COMPUTER_TITLE = "Computer couldn’t start.";

const RULES: readonly Rule[] = [
  { test: /vm-host binary not found|PEGOLES_VM_HOST/i, scope: "computer", title: COMPUTER_TITLE,
    hint: "The helper that runs Pegoles’ computer isn’t installed on this Mac." },
  { test: /not_entitled|entitlement|com\.apple\.security\.virtualization/i, scope: "computer", title: COMPUTER_TITLE,
    hint: "This build of Pegoles isn’t allowed to use virtualization on this Mac." },
  { test: /no space|ENOSPC|disk full/i, scope: "computer", title: "Not enough disk space.",
    hint: "Pegoles’ computer needs a few gigabytes free." },
  { test: /image|download|checksum|sha256/i, scope: "computer", title: "Computer setup didn’t finish.",
    hint: "The one-time download of its workspace failed." },
  { test: /Live updates unavailable/i, scope: "general", title: "Live updates paused.",
    hint: "Pegoles will keep checking every few seconds." },
];

const SCOPE_DEFAULT: Record<ErrorScope, { title: string; hint?: string }> = {
  task: { title: "Couldn’t hand this to Pegoles.", hint: "Your text is still here." },
  computer: { title: "Pegoles’ computer needs attention." },
  general: { title: "Something went wrong." },
};

/** Tauri rejects with strings; JS failures arrive as Error. */
export function errorText(raw: unknown): string {
  if (raw instanceof Error) return raw.message;
  if (typeof raw === "string") return raw;
  try { return JSON.stringify(raw); } catch { return String(raw); }
}

export function humanizeError(raw: unknown, scope: ErrorScope): HumanError {
  const detail = errorText(raw).trim() || "No details were reported.";
  const rule = RULES.find((candidate) => candidate.test.test(detail) && (!candidate.scope || candidate.scope === scope || scope === "general"));
  if (rule) {
    return { scope: rule.scope ?? scope, title: rule.title, hint: rule.hint, detail, retryable: rule.retryable ?? true };
  }
  const fallback = SCOPE_DEFAULT[scope];
  return { scope, title: fallback.title, hint: fallback.hint, detail, retryable: true };
}

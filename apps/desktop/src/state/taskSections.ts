import type { AgentTask } from "../lib/tauri";

/**
 * How the sidebar orders work: what needs you first, then what Pegoles is
 * doing, then everything else by when it last changed. A task never jumps
 * between "Done" and "Not started" groups; only attention and live work
 * are lifted out of time order.
 */
export type SectionKey = "needs-you" | "working" | "today" | "yesterday" | "week" | "earlier";

export interface TaskSection {
  readonly key: SectionKey;
  readonly label: string;
  readonly tasks: readonly AgentTask[];
}

const LABEL: Record<SectionKey, string> = {
  "needs-you": "Needs you",
  working: "Working",
  today: "Today",
  yesterday: "Yesterday",
  week: "Previous 7 days",
  earlier: "Earlier",
};

const ORDER: readonly SectionKey[] = ["needs-you", "working", "today", "yesterday", "week", "earlier"];
const DAY_MS = 86_400_000;

function timeSection(iso: string, startOfToday: number): SectionKey {
  const at = new Date(iso).getTime();
  if (!Number.isFinite(at) || at >= startOfToday) return "today";
  if (at >= startOfToday - DAY_MS) return "yesterday";
  if (at >= startOfToday - 7 * DAY_MS) return "week";
  return "earlier";
}

export function sectionOf(task: AgentTask, startOfToday: number): SectionKey {
  if (task.status === "waiting_for_approval") return "needs-you";
  if (task.status === "running") return "working";
  return timeSection(task.updated_at, startOfToday);
}

/** Non-empty sections in display order; newest first inside each. */
export function taskSections(tasks: readonly AgentTask[], now: number = Date.now()): TaskSection[] {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  const buckets = new Map<SectionKey, AgentTask[]>();
  for (const task of tasks) {
    const key = sectionOf(task, start.getTime());
    buckets.set(key, [...(buckets.get(key) ?? []), task]);
  }
  return ORDER.flatMap((key) => {
    const list = buckets.get(key);
    if (!list?.length) return [];
    const sorted = [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at) || b.created_at.localeCompare(a.created_at));
    return [{ key, label: LABEL[key], tasks: sorted }];
  });
}

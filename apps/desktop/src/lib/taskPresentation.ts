import type { StatusTone, TaskStatusWire } from "@pegoles/ui";

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/** Words + tone for a task status. Status is never blue and never color-only. */
export const TASK_STATUS: Record<TaskStatusWire, { label: string; tone: StatusTone }> = {
  pending: { label: "Pending", tone: "waiting" },
  running: { label: "Running", tone: "active" },
  waiting_for_approval: { label: "Needs approval", tone: "warning" },
  completed: { label: "Completed", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
  cancelled: { label: "Cancelled", tone: "neutral" },
};

/** Supporting line under a task title: what the status means right now. */
export function taskDetail(status: TaskStatusWire): string {
  return status === "pending" ? "Execution unavailable in this version" : TASK_STATUS[status].label;
}

/** "Just now", "12 min ago", "3 h ago", "Yesterday", then a short date. */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const at = new Date(iso).getTime();
  if (!Number.isFinite(at)) return "";
  const diff = Math.max(0, now - at);
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  if (at >= startOfToday.getTime()) {
    if (diff < MINUTE_MS) return "Just now";
    if (diff < HOUR_MS) return `${Math.floor(diff / MINUTE_MS)} min ago`;
    return `${Math.floor(diff / HOUR_MS)} h ago`;
  }
  if (at >= startOfToday.getTime() - DAY_MS) return "Yesterday";
  return new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

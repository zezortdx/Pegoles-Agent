import { currentLocale, t } from "../i18n";
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/** "Just now", "12 min ago", "3 h ago", "Yesterday", then a short date. */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const at = new Date(iso).getTime();
  if (!Number.isFinite(at)) return "";
  const diff = Math.max(0, now - at);
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  if (at >= startOfToday.getTime()) {
    if (diff < MINUTE_MS) return t("Just now");
    if (diff < HOUR_MS) return t("{0} min ago", { "0": Math.floor(diff / MINUTE_MS) });
    return t("{0} h ago", { "0": Math.floor(diff / HOUR_MS) });
  }
  if (at >= startOfToday.getTime() - DAY_MS) return t("Yesterday");
  return new Date(at).toLocaleDateString(currentLocale(), { month: "short", day: "numeric" });
}

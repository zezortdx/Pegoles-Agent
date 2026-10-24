import { currentLocale, t } from "../i18n";
/** Presentation helpers shared by transcript artifacts. Pure. */

export function timeOf(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleTimeString(currentLocale(), { hour: "numeric", minute: "2-digit" });
}

export function basename(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** The real parent folder of a path ("/w/r/a.md" → "/w/r"). */
export function parentOf(path: string): string {
  const cut = path.replace(/\/+$/, "").lastIndexOf("/");
  return cut > 0 ? path.slice(0, cut) : "/";
}

/** A short, human duration for a time Core measured. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return "<1s";
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
}

export function plural(count: number, one: string, many = `${one}s`): string {
  // Only these built-in nouns are localized; caller-provided data is never a key.
  if (one === "action") return t(count === 1 ? "{count} action" : "{count} actions", { count });
  if (one === "task") return t(count === 1 ? "{count} task" : "{count} tasks", { count });
  if (one === "file") return t(count === 1 ? "{count} file" : "{count} files", { count });
  if (one === "step") return t(count === 1 ? "{count} step" : "{count} steps", { count });
  if (one === "earlier step") return t(count === 1 ? "{count} earlier step" : "{count} earlier steps", { count });
  return `${count} ${count === 1 ? one : many}`;
}

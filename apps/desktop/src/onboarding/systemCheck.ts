import type { SystemCheck } from "../lib/tauri";
import { formatBytes, formatMemory } from "../lib/format";

/**
 * The system check as people read it: one row per requirement, in plain
 * words, from Core's facts only. Colour never carries the meaning alone:
 * every row has a tone word for assistive tech and an icon.
 */
export type CheckTone = "ok" | "warn" | "action" | "blocked";

export type CheckId = "system" | "virtualization" | "memory" | "acceleration" | "disk" | "runtime";

export interface CheckRow {
  readonly id: CheckId;
  readonly title: string;
  readonly tone: CheckTone;
  /** A sentence when something needs attention or explaining. */
  readonly detail?: string;
  /** Facts for "Technical details" only. */
  readonly technical: string;
}

/** The one thing to offer next, if any. */
export type CheckFix =
  | { readonly kind: "enable-virtualization" }
  | { readonly kind: "restart" }
  | { readonly kind: "firmware" };

export interface CheckVerdict {
  readonly rows: readonly CheckRow[];
  /** Nothing blocks setup (warnings are fine). */
  readonly canContinue: boolean;
  readonly fix: CheckFix | null;
}

export const TONE_WORD: Record<CheckTone, string> = {
  ok: "Ready",
  warn: "Works, with a note",
  action: "Needs a step",
  blocked: "Needs attention",
};

/** "Mac" or "PC": how people call the computer they're on. */
export function computerWord(platform: SystemCheck["platform"]): string {
  return platform === "macos" ? "Mac" : "PC";
}

function systemRow(c: SystemCheck): CheckRow {
  const arch = c.architecture === "arm64" ? (c.platform === "macos" ? "Apple silicon" : "ARM64") : "64-bit";
  const technical = `${c.os_name}, ${c.architecture}`;
  if (c.os_supported && c.architecture_supported) return { id: "system", title: c.os_name, tone: "ok", technical };
  const needs = c.platform === "macos"
    ? `Pegoles needs ${c.os_minimum} or later on a Mac with Apple silicon.`
    : c.platform === "windows"
      ? `Pegoles needs ${c.os_minimum} on a 64-bit Intel or AMD processor.`
      : "Pegoles runs on macOS and Windows.";
  return { id: "system", title: `${c.os_name} · ${arch}`, tone: "blocked", detail: needs, technical };
}

function virtualizationRow(c: SystemCheck): CheckRow {
  const why = "Pegoles uses hardware virtualization to give the AI its own isolated computer.";
  const technical = c.virtualization.technical;
  switch (c.virtualization.state) {
    case "ready":
      return { id: "virtualization", title: "Virtualization", tone: "ok", technical };
    case "needs_enable":
      return {
        id: "virtualization", title: "Virtualization needs to be turned on", tone: c.virtualization.fixable ? "action" : "blocked", technical,
        detail: c.virtualization.fixable ? `${why} Windows can turn it on for you.` : `${why} It's turned off on this ${computerWord(c.platform)}.`,
      };
    case "restart_pending":
      return {
        id: "virtualization", title: "One restart needed", tone: "action", technical,
        detail: "Virtualization is turned on. Windows needs to restart once before Pegoles can use it.",
      };
    case "firmware_disabled":
      return {
        id: "virtualization", title: "Virtualization needs to be enabled", tone: "blocked", technical,
        detail: `${why} It's currently disabled in this ${computerWord(c.platform)}'s firmware settings, which only you can change.`,
      };
    case "unsupported":
      return {
        id: "virtualization", title: "Virtualization isn't available", tone: "blocked", technical,
        detail: `${why} This ${computerWord(c.platform)} can't run it.`,
      };
    case "unknown":
      return {
        id: "virtualization", title: "Virtualization", tone: "warn", technical,
        detail: "Pegoles couldn't confirm it yet. Setup checks again when it starts Pegoles' computer.",
      };
  }
}

function memoryRow(c: SystemCheck): CheckRow {
  const title = `${formatMemory(c.memory_bytes)} memory`;
  const technical = `${c.memory_bytes} bytes installed; minimum ${formatMemory(c.memory_minimum_bytes)}, recommended ${formatMemory(c.memory_recommended_bytes)}`;
  if (c.memory_bytes >= c.memory_recommended_bytes) return { id: "memory", title, tone: "ok", technical };
  if (c.memory_bytes >= c.memory_minimum_bytes) {
    return { id: "memory", title, tone: "warn", technical, detail: "Enough to run Pegoles. Closing other apps helps while it works." };
  }
  return {
    id: "memory", title, tone: "warn", technical,
    detail: `Pegoles works best with at least ${formatMemory(c.memory_minimum_bytes)}. It may be slow or stop on this ${computerWord(c.platform)}.`,
  };
}

function accelerationRow(c: SystemCheck): CheckRow {
  const { kind, device, technical } = c.acceleration;
  switch (kind) {
    case "metal": case "cuda": case "vulkan":
      return { id: "acceleration", title: "Hardware acceleration", tone: "ok", technical, detail: device ? `Uses ${device}.` : undefined };
    case "cpu":
      return {
        id: "acceleration", title: "No graphics acceleration", tone: "warn", technical,
        detail: "The AI model will run on the processor. It works, but each step takes longer.",
      };
    case "none":
      return {
        id: "acceleration", title: "The local AI model can't run here", tone: "blocked", technical,
        detail: c.platform === "macos" ? "Pegoles Local needs a Mac with Apple silicon." : "This computer has no supported way to run the model.",
      };
  }
}

function diskRow(c: SystemCheck): CheckRow {
  const technical = `free ${c.disk_free_bytes ?? "unknown"} bytes, still needed ${c.disk_needed_bytes} bytes`;
  if (c.disk_needed_bytes === 0) return { id: "disk", title: "Everything is already downloaded", tone: "ok", technical };
  if (c.disk_free_bytes === null) {
    return { id: "disk", title: "Disk space", tone: "warn", technical, detail: `Pegoles needs about ${formatBytes(c.disk_needed_bytes)} free. It couldn't check how much there is.` };
  }
  if (c.disk_free_bytes >= c.disk_needed_bytes) {
    return { id: "disk", title: "Enough disk space", tone: "ok", technical, detail: `${formatBytes(c.disk_free_bytes)} available, about ${formatBytes(c.disk_needed_bytes)} needed.` };
  }
  return {
    id: "disk", title: "Not enough disk space", tone: "blocked", technical,
    detail: `Pegoles needs about ${formatBytes(c.disk_needed_bytes)} free and ${formatBytes(c.disk_free_bytes)} is available. Free up ${formatBytes(c.disk_needed_bytes - c.disk_free_bytes)}, then check again.`,
  };
}

function runtimeRow(c: SystemCheck): CheckRow {
  if (c.runtime_ready) return { id: "runtime", title: "AI runtime included", tone: "ok", technical: "bundled local inference runtime present" };
  return {
    id: "runtime", title: "The AI runtime is missing", tone: "blocked", technical: c.runtime_problem ?? "runtime not found",
    detail: "Part of Pegoles didn't install correctly. Installing Pegoles again fixes it.",
  };
}

export function checkVerdict(c: SystemCheck): CheckVerdict {
  const rows = [systemRow(c), virtualizationRow(c), memoryRow(c), accelerationRow(c), diskRow(c), runtimeRow(c)];
  // The runtime row repeats what "can't run here" already says on an unsupported Mac.
  const shown = c.acceleration.kind === "none" ? rows.filter((row) => row.id !== "runtime") : rows;
  const canContinue = shown.every((row) => row.tone === "ok" || row.tone === "warn");
  const v = c.virtualization;
  const fix: CheckFix | null = v.state === "needs_enable" && v.fixable ? { kind: "enable-virtualization" }
    : v.state === "restart_pending" ? { kind: "restart" }
      : v.state === "firmware_disabled" ? { kind: "firmware" }
        : null;
  return { rows: shown, canContinue, fix };
}

/** Everything under "Technical details", copyable as one block. */
export function technicalReport(c: SystemCheck, verdict: CheckVerdict): string {
  return [
    `Platform: ${c.platform} (${c.architecture})`,
    ...verdict.rows.map((row) => `${row.id}: ${row.tone} — ${row.technical}`),
  ].join("\n");
}

/**
 * DESIGN LAB FIXTURES — simulated data, allowed ONLY under /dev/design.
 * Production screens render real Core state; nothing here may be
 * imported outside apps/desktop/src/dev/.
 */
import type { BootStage, EffectsTier, ViewportState } from "@pegoles/ui";

export interface GovernorFixture {
  readonly id: "high" | "lowEnd" | "constrained" | "none";
  readonly label: string;
  readonly machine: string;
  readonly recommended: EffectsTier | null;
}

/** Mirrors what the Rust ResourceGovernor would recommend (Stream B). */
export const GOVERNOR_FIXTURES: readonly GovernorFixture[] = [
  { id: "high", label: "24 GB", machine: "M4 Pro · 24 GB · 12 cores", recommended: "full" },
  { id: "lowEnd", label: "8 GB", machine: "8 GB · 4 cores", recommended: "reduced" },
  { id: "constrained", label: "4 GB", machine: "4 GB · 2 cores", recommended: "minimal" },
  { id: "none", label: "Pending", machine: "Governor not answered yet", recommended: null },
];

/** Canonical startup chain (see PHASE 4 readiness chain). */
export const BOOT_STAGE_LABELS = [
  { id: "image", label: "Pegoles Base Image" },
  { id: "vm", label: "Virtual machine" },
  { id: "guest", label: "Guest runtime" },
  { id: "session", label: "Graphical session" },
  { id: "display", label: "Display" },
] as const;

const STAGE_DETAILS: Readonly<Record<string, string>> = {
  image: "verified",
  vm: "0.9 s",
  guest: "3.1 s",
  session: "1.4 s",
  display: "0.2 s",
};

/** Stages as they would look while `state` is current. */
export function stagesFor(state: ViewportState): BootStage[] {
  const activeIndex: Partial<Record<ViewportState, number>> = {
    preparing: 0,
    starting: 1,
    guest_connecting: 2,
    display_starting: 4,
  };
  const index = activeIndex[state];
  if (index === undefined) return [];
  return BOOT_STAGE_LABELS.map((stage, i) => {
    if (i < index) return { ...stage, status: "done" as const, detail: STAGE_DETAILS[stage.id] };
    if (i === index) {
      // Only the image download has a real byte measure.
      return stage.id === "image"
        ? { ...stage, status: "active" as const, progress: 0.62, detail: "684 MB of 1.1 GB" }
        : { ...stage, status: "active" as const };
    }
    return { ...stage, status: "pending" as const };
  });
}

export const ERROR_FIXTURE = "The guest runtime stopped answering (vsock closed).";

export interface ActivityFixture {
  readonly id: string;
  readonly kind: "task" | "agent" | "computer" | "guest" | "display" | "control" | "image" | "error" | "info";
  readonly title: string;
  readonly detail?: string;
  readonly at: string;
  readonly tone?: "success" | "danger" | "warning" | "active" | "user" | "neutral";
  readonly repeatCount?: number;
  readonly emphasis?: "quiet";
}

export const ACTIVITY_FIXTURES: readonly ActivityFixture[] = [
  { id: "a1", kind: "control", title: "Returned to Pegoles", at: "2026-09-21T14:12:40Z" },
  { id: "a2", kind: "control", title: "You took control", detail: "Keyboard and pointer routed to Pegoles Computer", at: "2026-09-21T14:09:05Z", tone: "user" },
  { id: "a3", kind: "guest", title: "Guest runtime reconnected", detail: "vsock handshake · protocol 3", at: "2026-09-21T14:07:51Z", repeatCount: 3, emphasis: "quiet" },
  { id: "a4", kind: "display", title: "Display ready", detail: "1440 × 900 · 5.6 s after start", at: "2026-09-21T14:02:18Z", tone: "success" },
  { id: "a5", kind: "display", title: "Graphical session ready", detail: "Weston answered a Wayland round-trip", at: "2026-09-21T14:02:18Z", emphasis: "quiet" },
  { id: "a6", kind: "guest", title: "Debian ready", detail: "Guest runtime ready in 4.0 s", at: "2026-09-21T14:02:16Z", tone: "success" },
  { id: "a7", kind: "computer", title: "Pegoles Computer started", at: "2026-09-21T14:02:12Z" },
  { id: "a8", kind: "error", title: "Display stopped responding", detail: "Recovered after restart", at: "2026-09-21T13:48:02Z" },
];

/** What the simulated agent is "doing" in the command demo. */
export const TASK_STEPS = [
  "Starting Pegoles Computer",
  "Opening a terminal",
  "Reading disk usage",
  "Summarising the result",
] as const;

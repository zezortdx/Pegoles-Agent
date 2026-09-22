/**
 * ComputerViewport vocabulary: what each Core-derived `ViewportState`
 * looks like and says. Pure data — the UI never guesses the state; it is
 * derived by Pegoles Core (`pegoles_protocol::ViewportState`, snake_case
 * on the wire) and passed in as-is.
 */
import type { StatusTone } from "./StatusIndicator.js";

export const VIEWPORT_STATES = [
  "off",
  "preparing",
  "starting",
  "guest_connecting",
  "display_starting",
  "ready",
  "paused",
  "agent_active",
  "user_controlled",
  "error",
] as const;

export type ViewportState = (typeof VIEWPORT_STATES)[number];

/** Energy edge treatment around the framebuffer slot. */
export type ViewportEnergy = "none" | "boot" | "ready" | "active" | "user" | "paused" | "error";

export interface ViewportStateSpec {
  /** Short status label (status pill). */
  readonly label: string;
  /** Sentence for the polite live region. */
  readonly announcement: string;
  readonly tone: StatusTone;
  readonly energy: ViewportEnergy;
  /** Part of the startup sequence (boot stages are relevant). */
  readonly booting: boolean;
  /** A human may take control from this state. */
  readonly canTakeControl: boolean;
}

export const VIEWPORT_STATE_SPECS: Readonly<Record<ViewportState, ViewportStateSpec>> = {
  off: {
    label: "Off",
    announcement: "Pegoles Computer is off.",
    tone: "neutral",
    energy: "none",
    booting: false,
    canTakeControl: false,
  },
  preparing: {
    label: "Preparing",
    announcement: "Preparing Pegoles Computer.",
    tone: "active",
    energy: "boot",
    booting: true,
    canTakeControl: false,
  },
  starting: {
    label: "Starting",
    announcement: "Pegoles Computer is starting.",
    tone: "active",
    energy: "boot",
    booting: true,
    canTakeControl: false,
  },
  guest_connecting: {
    label: "Connecting",
    announcement: "Connecting to Pegoles Computer's runtime.",
    tone: "active",
    energy: "boot",
    booting: true,
    canTakeControl: false,
  },
  display_starting: {
    label: "Starting display",
    announcement: "Starting Pegoles Computer's display.",
    tone: "active",
    energy: "boot",
    booting: true,
    canTakeControl: false,
  },
  ready: {
    label: "Ready",
    announcement: "Pegoles Computer is ready.",
    tone: "success",
    energy: "ready",
    booting: false,
    canTakeControl: true,
  },
  paused: {
    label: "Paused",
    announcement: "Pegoles Computer is paused.",
    tone: "paused",
    energy: "paused",
    booting: false,
    canTakeControl: false,
  },
  agent_active: {
    label: "Pegoles is working",
    announcement: "Pegoles is using its computer.",
    tone: "active",
    energy: "active",
    booting: false,
    canTakeControl: true,
  },
  user_controlled: {
    label: "You're in control",
    announcement:
      "You're controlling Pegoles Computer. Your keyboard and pointer go to it until you return to Pegoles.",
    tone: "user",
    energy: "user",
    booting: false,
    canTakeControl: false,
  },
  error: {
    label: "Needs attention",
    announcement: "Pegoles Computer hit a problem.",
    tone: "danger",
    energy: "error",
    booting: false,
    canTakeControl: false,
  },
};

export type BootStageStatus = "pending" | "active" | "done" | "failed" | "skipped";

/**
 * One REAL startup stage, mapped by the app from Core events (e.g. VM
 * running → guest runtime ready → graphical session ready → display
 * attached → display ready). `progress` exists only when a real measure
 * exists (image download bytes); otherwise the stage is indeterminate.
 */
export interface BootStage {
  readonly id: string;
  readonly label: string;
  readonly status: BootStageStatus;
  /** Real detail, e.g. "1.2 s" or "412 MB of 1.1 GB". */
  readonly detail?: string;
  /** Real fraction 0..1, or null/undefined when unknown. */
  readonly progress?: number | null;
}

export const BOOT_STAGE_STATUS_TEXT: Readonly<Record<BootStageStatus, string>> = {
  pending: "Pending",
  active: "In progress",
  done: "Done",
  failed: "Failed",
  skipped: "Skipped",
};

/** The stage to surface in compact layouts: failed › active › last done. */
export function currentBootStage(stages: readonly BootStage[]): BootStage | null {
  return (
    stages.find((s) => s.status === "failed") ??
    stages.find((s) => s.status === "active") ??
    [...stages].reverse().find((s) => s.status === "done") ??
    null
  );
}

export interface DisplaySize {
  readonly width_px: number;
  readonly height_px: number;
}

/** DesktopLarge (pegoles_protocol DisplayConfig preset). */
export const DEFAULT_DISPLAY: DisplaySize = { width_px: 1440, height_px: 900 };

export function displayAspect(display: DisplaySize): number {
  const w = display.width_px > 0 ? display.width_px : DEFAULT_DISPLAY.width_px;
  const h = display.height_px > 0 ? display.height_px : DEFAULT_DISPLAY.height_px;
  return w / h;
}

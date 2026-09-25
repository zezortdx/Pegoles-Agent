/**
 * Pegoles Computer is one surface with spatial levels. Closed, it lives in
 * the sidebar and the toolbar; Side shares the window with the task; Focus
 * gives it most of the room and keeps the task readable beside it; Full
 * hands it the whole window. Widths are plain pixels from the window width
 * (never from a measured element), so the columns can interpolate without
 * feeding back into themselves.
 */
export type ComputerLevel = "side" | "focus" | "full";

export const SIDEBAR_WIDTH = 260;
/** The narrowest the task column may get beside the computer. */
export const WORK_MIN = 360;
const SIDE_MIN = 380;
const SIDE_MAX = 680;
const SIDE_SHARE = 0.42;
const FOCUS_WORK_MIN = 340;
const FOCUS_WORK_MAX = 420;
const FOCUS_WORK_SHARE = 0.3;

export interface Columns {
  readonly sidebar: number;
  readonly computer: number;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export function columns(level: ComputerLevel | null, windowWidth: number, sidebarShown: boolean): Columns {
  if (level === "full") return { sidebar: 0, computer: Math.max(0, Math.round(windowWidth)) };
  const sidebar = sidebarShown ? SIDEBAR_WIDTH : 0;
  const available = Math.max(0, windowWidth - sidebar);
  if (level === null) return { sidebar, computer: 0 };
  if (level === "focus") {
    const work = clamp(Math.round(available * FOCUS_WORK_SHARE), FOCUS_WORK_MIN, FOCUS_WORK_MAX);
    return { sidebar, computer: Math.max(0, available - work) };
  }
  const side = clamp(Math.round(available * SIDE_SHARE), SIDE_MIN, SIDE_MAX);
  return { sidebar, computer: Math.max(0, Math.min(side, available - WORK_MIN)) };
}

/** One step back toward the task: Full → Focus → Side → closed. */
export function stepBack(level: ComputerLevel): ComputerLevel | null {
  return level === "full" ? "focus" : level === "focus" ? "side" : null;
}

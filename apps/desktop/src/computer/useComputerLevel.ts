import { useCallback, useEffect, useRef, useState } from "react";
import type { ComputerLevel } from "./layout";

/** Matches --dur-spatial, plus a frame of slack for the last transitionend. */
export const SPATIAL_MS = 460;

export type LevelMotion = "open" | "close" | "resize";

export interface ComputerLevelState {
  /** Where the computer is going (null: closed). Drives the columns. */
  readonly level: ComputerLevel | null;
  /** What the panel renders: stays mounted at its last level while it closes. */
  readonly mounted: ComputerLevel | null;
  /** The workspace is recomposing; null once it has settled. */
  readonly motion: LevelMotion | null;
  readonly setLevel: (next: ComputerLevel | null) => void;
}

/**
 * One computer, several spatial levels. A level change starts one spatial
 * motion of the columns; the panel stays mounted until a close has
 * finished, so it retracts the way it came instead of vanishing.
 */
export function useComputerLevel(animated: boolean): ComputerLevelState {
  const [level, setLevelState] = useState<ComputerLevel | null>(null);
  const [mounted, setMounted] = useState<ComputerLevel | null>(null);
  const [motion, setMotion] = useState<LevelMotion | null>(null);
  const timer = useRef(0);
  const current = useRef<ComputerLevel | null>(null);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const setLevel = useCallback((next: ComputerLevel | null) => {
    const previous = current.current;
    if (next === previous) return;
    current.current = next;
    window.clearTimeout(timer.current);
    setLevelState(next);
    if (next !== null) setMounted(next);
    if (!animated) {
      setMotion(null);
      if (next === null) setMounted(null);
      return;
    }
    setMotion(previous === null ? "open" : next === null ? "close" : "resize");
    timer.current = window.setTimeout(() => {
      setMotion(null);
      if (current.current === null) setMounted(null);
    }, SPATIAL_MS);
  }, [animated]);

  return { level, mounted, motion, setLevel };
}

import { useCallback, useEffect, useRef, useState } from "react";

/**
 * A real picture of its screen when the live native view isn't available:
 * captured on demand from the guest, refreshed when Pegoles acts on the
 * machine (at most every MIN_INTERVAL_MS) and slowly otherwise, and only
 * while someone can see it. Never a mock: no capture, no picture.
 */
export const MIN_INTERVAL_MS = 1500;
export const IDLE_REFRESH_MS = 8000;

export interface Snapshot {
  readonly src: string | null;
  /** When the picture was taken (ms since epoch). */
  readonly at: number | null;
  readonly error: string | null;
  readonly loading: boolean;
  readonly refresh: () => void;
}

export interface SnapshotOptions {
  readonly enabled: boolean;
  /** Changes whenever Pegoles did something that may have changed the screen. */
  readonly trigger: number;
  readonly capture: () => Promise<{ png_base64: string }>;
}

export function useScreenSnapshot({ enabled, trigger, capture }: SnapshotOptions): Snapshot {
  const [state, setState] = useState<{ src: string | null; at: number | null; error: string | null; loading: boolean }>(
    { src: null, at: null, error: null, loading: false },
  );
  const inFlight = useRef(false);
  const last = useRef(0);
  const timer = useRef(0);
  const live = useRef(enabled);
  live.current = enabled;
  const captureRef = useRef(capture);
  captureRef.current = capture;

  const take = useCallback(() => {
    if (!live.current || inFlight.current || document.hidden) return;
    inFlight.current = true;
    last.current = Date.now();
    setState((previous) => ({ ...previous, loading: true }));
    captureRef.current()
      .then((frame) => {
        if (!live.current) return;
        setState({ src: `data:image/png;base64,${frame.png_base64}`, at: Date.now(), error: null, loading: false });
      })
      .catch((reason: unknown) => {
        setState((previous) => ({ ...previous, error: String(reason), loading: false }));
      })
      .finally(() => { inFlight.current = false; });
  }, []);

  /** Coalesce requests into one capture no sooner than MIN_INTERVAL_MS after the last. */
  const schedule = useCallback(() => {
    if (timer.current) return;
    const wait = Math.max(0, last.current + MIN_INTERVAL_MS - Date.now());
    timer.current = window.setTimeout(() => { timer.current = 0; take(); }, wait);
  }, [take]);

  useEffect(() => {
    if (!enabled) return;
    schedule();
    const idle = window.setInterval(schedule, IDLE_REFRESH_MS);
    const onVisible = () => { if (!document.hidden) schedule(); };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(idle);
      window.clearTimeout(timer.current);
      timer.current = 0;
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [enabled, schedule]);

  useEffect(() => {
    if (enabled) schedule();
  }, [trigger, enabled, schedule]);

  const refresh = useCallback(() => { last.current = 0; window.clearTimeout(timer.current); timer.current = 0; take(); }, [take]);
  return { ...state, refresh };
}

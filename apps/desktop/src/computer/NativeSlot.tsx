import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { api } from "../lib/tauri";

// Serialize native mutations so a late resize can never reattach a departed view.
let displayQueue: Promise<unknown> = Promise.resolve();
function enqueue(action: () => Promise<unknown>) {
  const result = displayQueue.catch(() => undefined).then(action);
  displayQueue = result;
  return result;
}

/**
 * Geometry is read once per frame however many observers fire. When frames
 * stall (hidden or occluded window) this deadline still delivers the
 * latest rectangle, so a hide is never lost.
 */
export const MEASURE_MAX_WAIT_MS = 150;

/**
 * The page never animates the slot, so the native view never animates
 * either: it jumps to where the slot already is. Panel and full-view
 * motion hide the view (`obscured`) and reveal it once settled instead.
 */
const NATIVE_ANIMATE_MS = 0;

export interface NativeSlotProps {
  /** A native framebuffer view may attach over this slot. */
  readonly enabled: boolean;
  readonly onError: (error: unknown) => void;
  /** Something covers or is still moving the slot (a drawer, the panel
   * arriving): hide the native view without detaching it, so a person's
   * control session survives. */
  readonly obscured?: boolean;
  /** Guest framebuffer size; the slot keeps its aspect ratio. */
  readonly display?: { width_px: number; height_px: number } | null;
  /** Shown inside the slot when no native view covers it. */
  readonly children?: ReactNode;
}

/**
 * The hole the native VZ view sits in. The DOM never draws over it and no
 * ancestor may transform it; geometry is measured after commit and only
 * changed rectangles cross IPC. Unmounting detaches the view, which also
 * ends a person's control session: keep this mounted across preview and
 * full view.
 */
export function NativeSlot({ enabled, onError, obscured = false, display, children }: NativeSlotProps) {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  const measureRef = useRef<(() => void) | null>(null);
  // Callers may pass a new callback each render; it must never re-run attach.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const obscuredRef = useRef(obscured);
  obscuredRef.current = obscured;
  // Banners, handoff bars and `obscured` can change without a resize.
  useLayoutEffect(() => { measureRef.current?.(); });
  useEffect(() => {
    if (!enabled || !slot) return;
    let cancelled = false;
    let frame = 0;
    let deadline = 0;
    let lastGeometry = "";
    const report = (error: unknown) => { if (!cancelled) onErrorRef.current(error); };
    const flush = () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(deadline);
      frame = 0;
      deadline = 0;
      if (cancelled) return;
      const { x, y, width, height } = slot.getBoundingClientRect();
      const visible = !document.hidden && !obscuredRef.current && width > 0 && height > 0;
      const key = JSON.stringify([x, y, width, height, visible]);
      if (key === lastGeometry) return;
      lastGeometry = key;
      void enqueue(() => cancelled ? Promise.resolve() : api.setDisplayGeometry({
        rect: { x, y, width, height },
        visible,
        animate_ms: NATIVE_ANIMATE_MS,
      })).catch((error: unknown) => { if (lastGeometry === key) lastGeometry = ""; report(error); });
    };
    // Coalesce: the first request schedules one read; later ones ride along.
    const measure = () => {
      if (frame || deadline) return;
      frame = window.requestAnimationFrame(flush);
      deadline = window.setTimeout(flush, MEASURE_MAX_WAIT_MS);
    };
    const observer = new ResizeObserver(measure);
    measureRef.current = measure;
    observer.observe(slot);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    document.addEventListener("visibilitychange", measure);
    // Shell layout transitions (sidebar fold, inspector column) can move the slot without resizing it.
    document.addEventListener("transitionend", measure, true);
    measure();
    return () => {
      cancelled = true;
      measureRef.current = null;
      window.cancelAnimationFrame(frame);
      window.clearTimeout(deadline);
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
      document.removeEventListener("visibilitychange", measure);
      document.removeEventListener("transitionend", measure, true);
      void enqueue(api.detachDisplay).catch((error: unknown) => onErrorRef.current(error));
    };
  }, [enabled, slot]);

  const style = display ? ({ aspectRatio: `${display.width_px} / ${display.height_px}` } as CSSProperties) : undefined;
  return (
    <div ref={setSlot} className="native-slot" data-framebuffer-slot="" data-native={enabled || undefined} style={style}>
      {!enabled && children}
    </div>
  );
}

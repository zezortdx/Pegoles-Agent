import { useEffect, useState } from "react";
import { ComputerViewport, useFluxGlass, type ComputerViewportProps } from "@pegoles/ui";
import { api } from "../lib/tauri";

// Serialize native mutations so a late resize can never reattach a departed view.
let displayQueue: Promise<unknown> = Promise.resolve();
function enqueue(action: () => Promise<unknown>) {
  const result = displayQueue.catch(() => undefined).then(action);
  displayQueue = result;
  return result;
}

export function NativeComputer({ enabled, onError, ...props }: ComputerViewportProps & {
  enabled: boolean;
  onError: (message: string) => void;
}) {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  const { reducedMotion, tier } = useFluxGlass();
  useEffect(() => {
    if (!enabled || !slot) return;
    let cancelled = false;
    let timer = 0;
    const report = (error: unknown) => { if (!cancelled) onError(String(error)); };
    const measure = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const { x, y, width, height } = slot.getBoundingClientRect();
        void enqueue(() => cancelled ? Promise.resolve() : api.setDisplayGeometry({
          rect: { x, y, width, height },
          visible: !document.hidden && width > 0 && height > 0,
          animate_ms: reducedMotion || tier === "minimal" ? 0 : 420,
        })).catch(report);
      }, 100);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(slot);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    document.addEventListener("visibilitychange", measure);
    measure();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
      document.removeEventListener("visibilitychange", measure);
      void enqueue(api.detachDisplay).catch(onError);
    };
  }, [enabled, slot, reducedMotion, tier, onError]);
  return <ComputerViewport {...props} nativeSurface={enabled} slotRef={setSlot} />;
}

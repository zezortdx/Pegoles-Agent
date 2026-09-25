import { useEffect, useState } from "react";

/**
 * The current time, re-read every `intervalMs` while `active` — for elapsed
 * labels on live work. Idle screens never tick.
 */
export function useNow(active: boolean, intervalMs = 15_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [active, intervalMs]);
  return now;
}

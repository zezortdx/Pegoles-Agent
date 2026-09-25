import { useEffect, useRef, useState } from "react";

/**
 * Show each value for at least `holdMs` so fast real changes stay readable.
 * Values are only ever delayed and coalesced (the latest wins), never made up.
 */
export function useHeldValue<T>(value: T, key: string, holdMs: number): T {
  const [shown, setShown] = useState({ key, value });
  const since = useRef(0);

  useEffect(() => {
    if (key === shown.key) return;
    const wait = Math.max(0, holdMs - (Date.now() - since.current));
    const timer = window.setTimeout(() => {
      since.current = Date.now();
      setShown({ key, value });
    }, wait);
    return () => window.clearTimeout(timer);
  }, [key, value, shown.key, holdMs]);

  useEffect(() => { since.current = Date.now(); }, []);

  // Same key: the freshest value (e.g. same line, new object identity).
  return key === shown.key ? value : shown.value;
}

import { useEffect, useState } from "react";

/** Quiet period after the last resize event before columns animate again. */
const SETTLE_MS = 160;

/**
 * The window's width, and whether the person is resizing it right now.
 * While resizing, column changes apply instantly instead of animating
 * behind the pointer.
 */
export function useWindowWidth(): { width: number; resizing: boolean } {
  const [width, setWidth] = useState(() => (typeof window === "undefined" ? 1440 : window.innerWidth));
  const [resizing, setResizing] = useState(false);
  useEffect(() => {
    let timer = 0;
    const onResize = () => {
      setWidth(window.innerWidth);
      setResizing(true);
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setResizing(false), SETTLE_MS);
    };
    window.addEventListener("resize", onResize);
    return () => { window.removeEventListener("resize", onResize); window.clearTimeout(timer); };
  }, []);
  return { width, resizing };
}

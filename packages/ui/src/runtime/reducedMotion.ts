import { useSyncExternalStore } from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

function mediaQuery(): MediaQueryList | null {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  return window.matchMedia(QUERY);
}

function subscribe(onChange: () => void): () => void {
  const mql = mediaQuery();
  if (!mql) return () => undefined;
  mql.addEventListener("change", onChange);
  return () => mql.removeEventListener("change", onChange);
}

function read(): boolean {
  return mediaQuery()?.matches ?? false;
}

/**
 * The OS reduced-motion preference, live. Read-only: Pegoles never
 * changes accessibility settings.
 */
export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, read, () => false);
}

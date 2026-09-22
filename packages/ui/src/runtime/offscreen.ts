/**
 * Per-element offscreen pausing through ONE shared IntersectionObserver.
 *
 * Observed elements get `data-offscreen="true|false"` written directly
 * (no React state), and CSS pauses their ambient/work animations:
 * `[data-offscreen="true"] .pg-ambient { animation-play-state: paused }`.
 * Where IntersectionObserver is unavailable (tests, very old engines)
 * elements are treated as onscreen.
 */
import { useCallback, useRef } from "react";

export type OffscreenListener = (offscreen: boolean) => void;

interface Entry {
  readonly listeners: Set<OffscreenListener>;
}

let observer: IntersectionObserver | null = null;
const entries = new Map<Element, Entry>();

function handle(records: IntersectionObserverEntry[]): void {
  for (const record of records) {
    const offscreen = !record.isIntersecting;
    const value = offscreen ? "true" : "false";
    if (record.target.getAttribute("data-offscreen") !== value) {
      record.target.setAttribute("data-offscreen", value);
    }
    const entry = entries.get(record.target);
    if (!entry) continue;
    for (const listener of entry.listeners) listener(offscreen);
  }
}

function getObserver(): IntersectionObserver | null {
  if (observer) return observer;
  if (typeof IntersectionObserver === "undefined") return null;
  // A small margin so loops resume just before they scroll into view.
  observer = new IntersectionObserver(handle, { rootMargin: "64px" });
  return observer;
}

/**
 * Observe `element`; returns an unobserve function. Listeners are
 * optional (CSS handles pausing through the attribute).
 */
export function observeOffscreen(element: Element, listener?: OffscreenListener): () => void {
  let entry = entries.get(element);
  if (!entry) {
    entry = { listeners: new Set() };
    entries.set(element, entry);
    getObserver()?.observe(element);
  }
  if (listener) entry.listeners.add(listener);
  return () => {
    const current = entries.get(element);
    if (!current) return;
    if (listener) current.listeners.delete(listener);
    if (current.listeners.size === 0) {
      entries.delete(element);
      observer?.unobserve(element);
      element.removeAttribute("data-offscreen");
    }
  };
}

/** Number of elements currently observed (diagnostics/tests). */
export function observedOffscreenCount(): number {
  return entries.size;
}

/** Test hook: drop the shared observer so a new global mock is picked up. */
export function resetOffscreenObserverForTests(): void {
  observer?.disconnect();
  observer = null;
  entries.clear();
}

/**
 * Ref callback that pauses the element's ambient animations while it is
 * offscreen. Stable identity; safe to attach to any element.
 */
export function useOffscreenPause<T extends Element>(listener?: OffscreenListener): (node: T | null) => void {
  const cleanup = useRef<(() => void) | null>(null);
  const listenerRef = useRef(listener);
  listenerRef.current = listener;
  return useCallback((node: T | null) => {
    cleanup.current?.();
    cleanup.current = null;
    if (node) {
      cleanup.current = observeOffscreen(node, (offscreen) => listenerRef.current?.(offscreen));
    }
  }, []);
}

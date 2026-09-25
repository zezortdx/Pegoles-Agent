import { useEffect } from "react";
import { api } from "../lib/tauri";

/**
 * Mirrors the system's Reduce Transparency and Increase Contrast settings
 * onto <html> (WKWebView doesn't expose them to CSS). Re-read whenever the
 * window regains focus, which is when a person comes back from System Settings.
 */
export function useAccessibilityDisplay(native: boolean): void {
  useEffect(() => {
    if (!native) return;
    const root = document.documentElement;
    let alive = true;
    const read = () => {
      api.accessibilityDisplay()
        .then((display) => {
          if (!alive) return;
          root.toggleAttribute("data-reduce-transparency", display.reduce_transparency);
          root.toggleAttribute("data-increase-contrast", display.increase_contrast);
        })
        // An older Core without the command: CSS media queries remain the fallback.
        .catch(() => undefined);
    };
    read();
    window.addEventListener("focus", read);
    return () => { alive = false; window.removeEventListener("focus", read); };
  }, [native]);
}

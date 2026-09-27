import { useCallback, useEffect, useState } from "react";
import { api, type OnboardingState } from "../lib/tauri";

export interface OnboardingGate {
  /** Onboarding to show, or null when it's done (or Core can't say). */
  readonly state: OnboardingState | null;
  readonly finish: () => Promise<void>;
}

interface Boot { readonly onboarding?: OnboardingState | null }

/** What Core set before the page's scripts ran (the desktop app only). */
function bootState(): OnboardingState | null {
  const boot = (window as unknown as { __PEGOLES_BOOT__?: Boot }).__PEGOLES_BOOT__;
  const value = boot?.onboarding;
  return value && typeof value === "object" && value.completed === false ? value : null;
}

/**
 * Whether this launch starts with onboarding. The first render uses the
 * state Core put on the page at startup (no flash of the main window, no
 * wait); Core is then asked again in case the page outlived it (a reload).
 * If Core can't answer, the main window opens rather than trapping the
 * person on a blank screen.
 */
export function useOnboarding(enabled: boolean): OnboardingGate {
  const [state, setState] = useState<OnboardingState | null>(() => (enabled ? bootState() : null));

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    Promise.resolve()
      .then(() => api.getOnboarding())
      .then((value) => { if (alive && value) setState(value.completed ? null : value); })
      .catch(() => undefined);
    return () => { alive = false; };
  }, [enabled]);

  const finish = useCallback(async () => {
    try { await api.finishOnboarding(); } finally { setState(null); }
  }, []);

  return { state, finish };
}

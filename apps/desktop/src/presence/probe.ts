/**
 * One-time WebGL2 capability probe plus a session kill switch. After
 * repeated context losses or a renderer crash the presence stays SVG for
 * the rest of the session.
 */
let probed: boolean | null = null;
let disabled = false;

export const MAX_CONTEXT_LOSSES = 2;

export function webgl2Available(): boolean {
  if (probed !== null) return probed;
  probed = detect();
  return probed;
}

function detect(): boolean {
  if (typeof document === "undefined" || typeof navigator === "undefined") return false;
  // jsdom has no canvas backend (and logs when asked for one).
  if (/jsdom/i.test(navigator.userAgent)) return false;
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2", { failIfMajorPerformanceCaveat: glCaveatCheck() });
    if (!gl) return false;
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return true;
  } catch {
    return false;
  }
}

/**
 * Production always refuses software/"major caveat" WebGL. Development
 * builds can force it with `?gl=force` in the hash query (headless captures).
 */
export function glCaveatCheck(): boolean {
  if (!import.meta.env.DEV || typeof window === "undefined") return true;
  return !/[?&]gl=force\b/.test(window.location.hash + window.location.search);
}

export function glDisabledForSession(): boolean {
  return disabled;
}

export function disableGlForSession(): void {
  disabled = true;
}

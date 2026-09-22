import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AmbientController } from "./ambient.js";

function setVisibility(state: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { value: state, configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
}

function attrs(el: HTMLElement) {
  return {
    ambient: el.getAttribute("data-ambient"),
    reason: el.getAttribute("data-ambient-reason"),
    tier: el.getAttribute("data-effects-tier"),
    reducedMotion: el.getAttribute("data-reduced-motion"),
    visibility: el.getAttribute("data-visibility"),
  };
}

describe("AmbientController", () => {
  let root: HTMLElement;
  let controller: AmbientController | null = null;

  const make = (overrides: Partial<ConstructorParameters<typeof AmbientController>[0]> = {}) => {
    controller = new AmbientController({
      root,
      tier: "full",
      reducedMotion: false,
      idleAfterMs: 10_000,
      initiallyFocused: true,
      ...overrides,
    });
    controller.start();
    return controller;
  };

  beforeEach(() => {
    vi.useFakeTimers();
    setVisibility("visible");
    root = document.createElement("div");
    document.body.append(root);
  });

  afterEach(() => {
    controller?.stop();
    controller = null;
    root.remove();
    vi.useRealTimers();
  });

  it("runs when focused, visible, active and the tier allows ambient", () => {
    make();
    expect(attrs(root)).toEqual({
      ambient: "running",
      reason: "none",
      tier: "full",
      reducedMotion: "false",
      visibility: "visible",
    });
  });

  it("pauses on window blur and resumes on focus", () => {
    make();
    window.dispatchEvent(new Event("blur"));
    expect(attrs(root).ambient).toBe("paused");
    expect(attrs(root).reason).toBe("unfocused");
    window.dispatchEvent(new Event("focus"));
    expect(attrs(root).ambient).toBe("running");
  });

  it("pauses when the document is hidden (minimized / other Space) and resumes", () => {
    make();
    setVisibility("hidden");
    expect(attrs(root)).toMatchObject({ ambient: "paused", reason: "hidden", visibility: "hidden" });
    setVisibility("visible");
    expect(attrs(root)).toMatchObject({ ambient: "running", visibility: "visible" });
  });

  it("accepts an external hidden signal (Tauri minimize)", () => {
    const c = make();
    c.setExternallyHidden(true);
    expect(attrs(root)).toMatchObject({ ambient: "paused", reason: "hidden", visibility: "hidden" });
    c.setExternallyHidden(false);
    expect(attrs(root).ambient).toBe("running");
  });

  it("goes idle after the no-input interval and wakes on input", () => {
    make();
    vi.advanceTimersByTime(9_999);
    expect(attrs(root).ambient).toBe("running");
    vi.advanceTimersByTime(1);
    expect(attrs(root)).toMatchObject({ ambient: "paused", reason: "idle" });
    window.dispatchEvent(new Event("pointermove"));
    expect(attrs(root).ambient).toBe("running");
  });

  it("input during the interval postpones idle without resetting timers per event", () => {
    make();
    vi.advanceTimersByTime(6_000);
    window.dispatchEvent(new Event("keydown"));
    vi.advanceTimersByTime(6_000);
    expect(attrs(root).ambient).toBe("running");
    vi.advanceTimersByTime(4_000);
    expect(attrs(root).reason).toBe("idle");
  });

  it("never goes idle while busy (active work), and resumes the idle clock after", () => {
    const c = make({ busy: true });
    vi.advanceTimersByTime(60_000);
    expect(attrs(root).ambient).toBe("running");
    c.setBusy(false);
    vi.advanceTimersByTime(9_999);
    expect(attrs(root).ambient).toBe("running");
    vi.advanceTimersByTime(1);
    expect(attrs(root).reason).toBe("idle");
    c.setBusy(true);
    expect(attrs(root).ambient).toBe("running");
  });

  it("Minimal tier keeps ambient paused regardless of focus", () => {
    const c = make({ tier: "minimal" });
    expect(attrs(root)).toMatchObject({ ambient: "paused", reason: "tier", tier: "minimal" });
    c.setTier("reduced");
    expect(attrs(root)).toMatchObject({ ambient: "running", tier: "reduced" });
  });

  it("records reduced motion without pausing the gate or changing the tier", () => {
    const c = make();
    c.setReducedMotion(true);
    expect(attrs(root)).toMatchObject({ ambient: "running", tier: "full", reducedMotion: "true" });
  });

  it("stop() leaves ambient paused and removes listeners", () => {
    const c = make();
    c.stop();
    expect(attrs(root)).toMatchObject({ ambient: "paused", reason: "stopped" });
    window.dispatchEvent(new Event("focus"));
    expect(attrs(root).ambient).toBe("paused");
  });

  it("notifies subscribers only on real transitions", () => {
    const c = make();
    const listener = vi.fn();
    c.subscribe(listener);
    window.dispatchEvent(new Event("pointermove"));
    window.dispatchEvent(new Event("pointermove"));
    expect(listener).not.toHaveBeenCalled();
    window.dispatchEvent(new Event("blur"));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(c.snapshot()).toMatchObject({ running: false, reasons: ["unfocused"] });
  });
});

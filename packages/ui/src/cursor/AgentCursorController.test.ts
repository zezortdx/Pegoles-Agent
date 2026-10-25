import { describe, expect, it, vi } from "vitest";
import { AgentCursorController, DOUBLE_GAP_MS, type AgentCursorElements } from "./AgentCursorController.js";
import { GLIDE } from "./glide.js";
import { createFakeClock, type FakeClock } from "./testClock.js";

interface Played {
  readonly el: string;
  readonly frames: Keyframe[];
  readonly options: KeyframeAnimationOptions;
}

function node(name: string, log: Played[]): HTMLDivElement {
  const el = document.createElement("div");
  el.dataset.name = name;
  const running: { cancel: () => void }[] = [];
  Object.assign(el, {
    animate: vi.fn((frames: Keyframe[], options: KeyframeAnimationOptions) => {
      log.push({ el: name, frames, options });
      const anim = { cancel: vi.fn() };
      running.push(anim);
      return anim;
    }),
    getAnimations: () => running.splice(0),
  });
  return el;
}

function setup(options: { reducedMotion?: boolean; frame?: { width: number; height: number } } = {}) {
  const clock: FakeClock = createFakeClock();
  const log: Played[] = [];
  const elements: AgentCursorElements = {
    root: node("root", log),
    pointer: node("pointer", log),
    pointerBody: node("body", log),
    pulses: [node("pulse-a", log), node("pulse-b", log)],
    dragPath: node("drag", log),
    scrollCue: node("scroll", log),
  };
  const controller = new AgentCursorController({
    elements,
    scheduler: clock,
    reducedMotion: options.reducedMotion,
    devicePixelRatio: () => 2,
  });
  const frame = options.frame ?? { width: 800, height: 500 };
  controller.setFrame({ x: 0, y: 0, ...frame });
  return { controller, clock, elements, log };
}

/** Parsed translate of the pointer (px). */
function where(el: HTMLElement): { x: number; y: number } {
  const m = /translate3d\(([-\d.e]+)px, ([-\d.e]+)px/.exec(el.style.transform);
  if (!m) throw new Error(`no transform: ${el.style.transform}`);
  return { x: Number(m[1]), y: Number(m[2]) };
}

function translateOf(frames: Keyframe[]): string {
  return String(frames[0]?.transform ?? "");
}

describe("AgentCursorController", () => {
  it("appears where the first action happened, with no travel from nowhere", () => {
    const { controller, clock, elements } = setup();
    expect(controller.state).toBe("hidden");
    controller.moveTo({ x: 0.5, y: 0.5 });
    expect(controller.state).toBe("moving");
    expect(where(elements.pointer)).toEqual({ x: 400, y: 250 });
    expect(controller.isAnimating).toBe(false);
    expect(clock.pending).toBe(0);
  });

  it("glides between real coordinates with ease-in-out, then stops its loop", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.moveTo({ x: 0.9, y: 0.9 });
    expect(controller.isAnimating).toBe(true);
    const xs: number[] = [];
    let frames = 0;
    while (clock.pending > 0 && frames < 200) {
      clock.frame();
      xs.push(where(elements.pointer).x);
      frames += 1;
    }
    expect(where(elements.pointer)).toEqual({ x: 720, y: 450 });
    expect(controller.isAnimating).toBe(false);
    // Took a perceivable but short time, and moved smoothly: no teleport step.
    expect(frames * (1000 / 60)).toBeGreaterThan(150);
    expect(frames * (1000 / 60)).toBeLessThanOrEqual(GLIDE.maxMs + 20);
    const steps = xs.slice(1).map((x, i) => x - (xs[i] as number));
    expect(Math.max(...steps)).toBeLessThan(640 * 0.2);
    expect(steps.every((s) => s >= -1e-9)).toBe(true);
  });

  it("repeated moves each end exactly on their own target", () => {
    const { controller, clock, elements } = setup();
    for (const [x, y] of [[0.2, 0.3], [0.6, 0.3], [0.6, 0.8], [0.25, 0.75]] as const) {
      controller.moveTo({ x, y });
      clock.runUntilIdle();
      expect(where(elements.pointer)).toEqual({ x: Math.round(x * 800), y: Math.round(y * 500) });
    }
  });

  it("pulses a click at the exact click coordinate, when the cursor lands", () => {
    const { controller, clock, log } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.click({ x: 0.75, y: 0.5 });
    expect(controller.state).toBe("clicking");
    expect(log.filter((p) => p.el.startsWith("pulse"))).toHaveLength(0);
    clock.runUntilIdle();
    const pulses = log.filter((p) => p.el.startsWith("pulse"));
    expect(pulses).toHaveLength(1);
    expect(translateOf(pulses[0]!.frames)).toContain("translate3d(600px, 250px, 0)");
    expect(Number(pulses[0]!.options.duration)).toBeGreaterThanOrEqual(100);
    expect(Number(pulses[0]!.options.duration)).toBeLessThanOrEqual(250);
  });

  it("shows a double click as two quick pulses, not a special effect", () => {
    const { controller, clock, log } = setup();
    controller.click({ x: 0.4, y: 0.4 }, 2);
    clock.runUntilIdle();
    const pulses = log.filter((p) => p.el.startsWith("pulse"));
    expect(pulses.map((p) => p.el)).toEqual(["pulse-a", "pulse-b"]);
    expect(pulses[1]!.options.delay).toBe(DOUBLE_GAP_MS);
    expect(translateOf(pulses[1]!.frames)).toContain("translate3d(320px, 200px, 0)");
  });

  it("drags continuously from start to destination, pressed, with a path that fades after", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.1, y: 0.5 });
    controller.drag({ x: 0.2, y: 0.5 }, { x: 0.8, y: 0.5 }, 600);
    expect(controller.state).toBe("dragging");
    // Approach to the start point is quick.
    let approach = 0;
    while (!elements.root.hasAttribute("data-pressed") && approach < 60) {
      clock.frame();
      approach += 1;
    }
    expect(approach * (1000 / 60)).toBeLessThanOrEqual(220);
    expect(elements.dragPath.style.opacity).toBe("1");
    const xs: number[] = [];
    while (clock.pending > 0) {
      clock.frame();
      xs.push(where(elements.pointer).x);
    }
    // The stroke takes about the real drag's time and never jumps.
    expect(xs.length * (1000 / 60)).toBeGreaterThan(560);
    expect(xs.length * (1000 / 60)).toBeLessThan(660);
    expect(Math.max(...xs.slice(1).map((x, i) => x - (xs[i] as number)))).toBeLessThan(40);
    expect(where(elements.pointer)).toEqual({ x: 640, y: 250 });
    expect(elements.root.hasAttribute("data-pressed")).toBe(false);
    expect(elements.dragPath.style.opacity).toBe("0");
    expect(elements.dragPath.style.transition).toBe("");
  });

  it("retargets rapid actions from where it is: no backlog, no jump, converges to the newest", () => {
    const { controller, clock, elements, log } = setup();
    controller.moveTo({ x: 0.05, y: 0.05 });
    controller.click({ x: 0.3, y: 0.2 });
    clock.frame();
    clock.frame();
    let before = where(elements.pointer);
    for (let i = 0; i < 20; i += 1) {
      const target = { x: 0.1 + (i % 5) * 0.2, y: 0.9 - (i % 3) * 0.3 };
      controller.moveTo(target);
      expect(controller.backlog).toBe(0);
      clock.frame();
      const after = where(elements.pointer);
      // Continuity: one frame after a retarget the pointer is still next to where it was.
      expect(Math.hypot(after.x - before.x, after.y - before.y)).toBeLessThan(80);
      before = after;
    }
    // The interrupted click still showed, at its own coordinate, the moment it was superseded.
    const pulse = log.find((p) => p.el === "pulse-a");
    expect(pulse && translateOf(pulse.frames)).toContain("translate3d(240px, 100px, 0)");
    controller.moveTo({ x: 0.5, y: 0.5 });
    const frames = clock.runUntilIdle();
    expect(frames * (1000 / 60)).toBeLessThanOrEqual(GLIDE.maxMs + 20);
    expect(where(elements.pointer)).toEqual({ x: 400, y: 250 });
    expect(controller.backlog).toBe(0);
  });

  it("stops at once mid-glide: no motion, no effects, hidden, position forgotten", () => {
    const { controller, clock, elements, log } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.click({ x: 0.9, y: 0.9 });
    clock.frame();
    const frozen = elements.pointer.style.transform;
    controller.stop();
    expect(controller.state).toBe("hidden");
    expect(controller.isAnimating).toBe(false);
    expect(controller.backlog).toBe(0);
    expect(clock.pending).toBe(0);
    clock.frame();
    clock.frame();
    expect(elements.pointer.style.transform).toBe(frozen);
    expect(log.filter((p) => p.el.startsWith("pulse"))).toHaveLength(0);
    // After a stop (or reset) the next action appears fresh where it happens.
    controller.moveTo({ x: 0.2, y: 0.2 });
    expect(controller.isAnimating).toBe(false);
    expect(where(elements.pointer)).toEqual({ x: 160, y: 100 });
  });

  it("stop during a drag releases and removes the path immediately", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.2, y: 0.5 });
    controller.drag({ x: 0.2, y: 0.5 }, { x: 0.9, y: 0.5 }, 800);
    clock.frame();
    expect(elements.root.hasAttribute("data-pressed")).toBe(true);
    controller.stop();
    expect(elements.root.hasAttribute("data-pressed")).toBe(false);
    expect(elements.dragPath.style.opacity).toBe("0");
    expect(elements.dragPath.style.transition).toBe("none");
    expect(clock.pending).toBe(0);
  });

  it("follows a resize mid-glide: the same guest point, the new geometry", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.5, y: 0.5 });
    controller.moveTo({ x: 1, y: 1 });
    clock.frame();
    controller.setFrame({ x: 100, y: 40, width: 400, height: 250 });
    clock.runUntilIdle();
    expect(where(elements.pointer)).toEqual({ x: 500, y: 290 });
    controller.setFrame({ x: 0, y: 0, width: 1600, height: 1000 });
    expect(where(elements.pointer)).toEqual({ x: 1600, y: 1000 });
  });

  it("maps into a letterboxed frame for a different aspect ratio", () => {
    const { controller, elements } = setup();
    controller.setFrame({ x: 0, y: 175, width: 800, height: 450 });
    controller.moveTo({ x: 0.5, y: 0 });
    expect(where(elements.pointer)).toEqual({ x: 400, y: 175 });
  });

  it("under reduced motion lands at once and pulses without growing", () => {
    const { controller, clock, elements, log } = setup({ reducedMotion: true });
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.click({ x: 0.9, y: 0.5 });
    expect(clock.pending).toBe(0);
    expect(where(elements.pointer)).toEqual({ x: 720, y: 250 });
    const pulse = log.find((p) => p.el === "pulse-a");
    expect(pulse?.frames.map((f) => f.transform)).toEqual(["translate3d(720px, 250px, 0)", "translate3d(720px, 250px, 0)"]);
    expect(log.some((p) => p.el === "body")).toBe(false);
    controller.drag({ x: 0.1, y: 0.1 }, { x: 0.6, y: 0.6 }, 700);
    expect(clock.pending).toBe(0);
    expect(where(elements.pointer)).toEqual({ x: 480, y: 300 });
    expect(elements.root.getAttribute("data-rm")).toBe("true");
  });

  it("switching reduced motion on mid-glide lands immediately", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.moveTo({ x: 0.9, y: 0.1 });
    clock.frame();
    controller.setReducedMotion(true);
    expect(clock.pending).toBe(0);
    expect(where(elements.pointer)).toEqual({ x: 720, y: 50 });
  });

  it("expresses agent states without moving: observing, typing, thinking, done", () => {
    const { controller, clock, elements } = setup();
    controller.observe();
    expect(controller.state).toBe("hidden");
    controller.moveTo({ x: 0.3, y: 0.3 });
    const at = elements.pointer.style.transform;
    for (const [call, state] of [
      [() => controller.observe(), "observing"],
      [() => controller.typing(true), "typing"],
      [() => controller.typing(false), "thinking"],
      [() => controller.think(), "thinking"],
      [() => controller.done(), "done"],
    ] as const) {
      call();
      expect(controller.state).toBe(state);
      expect(elements.root.getAttribute("data-state")).toBe(state);
      expect(elements.pointer.style.transform).toBe(at);
      expect(clock.pending).toBe(0);
    }
  });

  it("a finished task lets the last glide land before settling", () => {
    const { controller, clock, elements, log } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.click({ x: 0.6, y: 0.6 });
    controller.done();
    expect(controller.state).toBe("done");
    clock.runUntilIdle();
    expect(where(elements.pointer)).toEqual({ x: 480, y: 300 });
    expect(log.filter((p) => p.el === "pulse-a")).toHaveLength(1);
  });

  it("scroll shows a small cue beside the pointer, in the scroll direction", () => {
    const { controller, clock, log } = setup();
    controller.scroll({ x: 0.5, y: 0.5 }, 0, 3);
    clock.runUntilIdle();
    const cue = log.find((p) => p.el === "scroll");
    expect(cue).toBeTruthy();
    expect(String(cue!.frames[1]!.transform)).toContain("rotate(0deg)");
    controller.scroll({ x: 0.5, y: 0.5 }, 0, -3);
    clock.runUntilIdle();
    expect(String(log.filter((p) => p.el === "scroll")[1]!.frames[1]!.transform)).toContain("rotate(180deg)");
  });

  it("a hidden window lands moves without animating", () => {
    const { controller, clock, elements } = setup();
    controller.moveTo({ x: 0.1, y: 0.1 });
    controller.moveTo({ x: 0.9, y: 0.9 });
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(clock.pending).toBe(0);
    expect(where(elements.pointer)).toEqual({ x: 720, y: 450 });
    controller.moveTo({ x: 0.2, y: 0.2 });
    expect(clock.pending).toBe(0);
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  });

  it("does nothing after dispose", () => {
    const { controller, clock } = setup();
    controller.dispose();
    controller.moveTo({ x: 0.5, y: 0.5 });
    controller.click({ x: 0.5, y: 0.5 });
    expect(controller.state).toBe("hidden");
    expect(clock.pending).toBe(0);
  });
});

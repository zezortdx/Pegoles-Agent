import { afterEach, describe, expect, it } from "vitest";
import { AgentCursorController, CLICK_MS, MAX_TRAIL_ELEMENTS, trailLengthFor } from "./AgentCursorController.js";
import { createFakeClock, type FakeClock } from "./testClock.js";

function setup(opts: { tier?: "full" | "reduced" | "minimal"; reducedMotion?: boolean } = {}) {
  const root = document.createElement("div");
  const make = () => {
    const n = document.createElement("div");
    root.appendChild(n);
    return n;
  };
  const elements = {
    root,
    pointer: make(),
    pointerBody: make(),
    ripple: make(),
    dragPath: make(),
    trail: Array.from({ length: MAX_TRAIL_ELEMENTS }, make),
  };
  document.body.appendChild(root);
  const clock: FakeClock = createFakeClock();
  const controller = new AgentCursorController({
    elements,
    scheduler: clock,
    timers: clock.timers,
    tier: opts.tier ?? "full",
    reducedMotion: opts.reducedMotion ?? false,
  });
  return { controller, clock, elements, root };
}

const pos = (c: AgentCursorController) => c.position;

let hidden = false;
Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });

afterEach(() => {
  hidden = false;
  document.body.innerHTML = "";
});

describe("AgentCursorController", () => {
  it("writes transforms straight to the DOM and arrives", async () => {
    const { controller, clock, elements } = setup();
    controller.show();
    const arrived = controller.moveTo(300, 200);
    expect(controller.state).toBe("moving");
    clock.frame();
    const mid = pos(controller);
    expect(mid.x).toBeGreaterThan(0);
    expect(mid.x).toBeLessThan(300);
    expect(elements.pointer.style.transform).toBe(`translate3d(${mid.x}px, ${mid.y}px, 0)`);
    clock.runUntilIdle();
    expect(pos(controller)).toEqual({ x: 300, y: 200 });
    expect(elements.pointer.style.transform).toBe("translate3d(300px, 200px, 0)");
    expect(controller.state).toBe("idle");
    await expect(arrived).resolves.toBe(true);
  });

  it("runs the rAF loop only while moving, and stops once settled", () => {
    const { controller, clock } = setup({ tier: "minimal" });
    controller.show();
    expect(controller.isAnimating).toBe(false);
    void controller.moveTo(120, 40);
    expect(controller.isAnimating).toBe(true);
    const frames = clock.runUntilIdle();
    expect(frames).toBeGreaterThan(5);
    expect(frames).toBeLessThan(120); // settles well under 2 s at 60 fps
    expect(controller.isAnimating).toBe(false);
    expect(clock.pending).toBe(0);
    const before = controller.frameCount;
    clock.frame();
    clock.frame();
    expect(controller.frameCount).toBe(before); // asleep: no frames while idle
  });

  it("keeps a short trail in Full that collapses and stops with the cursor", () => {
    const { controller, clock, elements } = setup({ tier: "full" });
    controller.show();
    void controller.moveTo(600, 0);
    clock.frame();
    clock.frame();
    const lead = Number(elements.trail[0]?.style.opacity);
    expect(lead).toBeGreaterThan(0);
    clock.runUntilIdle();
    expect(controller.isAnimating).toBe(false);
    for (const node of elements.trail) expect(Number(node.style.opacity || 0)).toBeLessThan(0.01);
  });

  it("uses tier-appropriate trail lengths", () => {
    expect(trailLengthFor("full", false)).toBeGreaterThanOrEqual(2);
    expect(trailLengthFor("full", false)).toBeLessThanOrEqual(MAX_TRAIL_ELEMENTS);
    expect(trailLengthFor("reduced", false)).toBeLessThanOrEqual(2);
    expect(trailLengthFor("minimal", false)).toBe(0);
    expect(trailLengthFor("full", true)).toBe(0);
    const { elements } = setup({ tier: "reduced" });
    expect(elements.trail.filter((n) => n.style.display !== "none")).toHaveLength(trailLengthFor("reduced", false));
  });

  it("reduced motion: no travel, no loop — appears at the destination", async () => {
    const { controller, clock } = setup({ reducedMotion: true });
    controller.show();
    const arrived = controller.moveTo(50, 60);
    expect(pos(controller)).toEqual({ x: 50, y: 60 });
    expect(controller.isAnimating).toBe(false);
    expect(clock.pending).toBe(0);
    await expect(arrived).resolves.toBe(true);
  });

  it("sleeps while the document is hidden (snaps instead of animating)", async () => {
    const { controller, clock } = setup();
    controller.show();
    const arrived = controller.moveTo(400, 400);
    clock.frame();
    hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(controller.isAnimating).toBe(false);
    expect(pos(controller)).toEqual({ x: 400, y: 400 });
    await expect(arrived).resolves.toBe(true);
    void controller.moveTo(10, 10);
    expect(controller.isAnimating).toBe(false);
    expect(clock.pending).toBe(0);
  });

  it("click is a brief state that ends on its own", () => {
    const { controller, clock, root } = setup();
    controller.show();
    controller.click();
    expect(controller.state).toBe("clicking");
    expect(root.getAttribute("data-state")).toBe("clicking");
    clock.timers.advance(CLICK_MS);
    expect(controller.state).toBe("idle");
  });

  it("drag draws a direction path that fades after the drag", async () => {
    const { controller, clock, elements } = setup();
    controller.show();
    const done = controller.dragTo(200, 0);
    expect(controller.state).toBe("dragging");
    expect(elements.dragPath.style.opacity).toBe("1");
    clock.frame();
    expect(elements.dragPath.style.transform).toMatch(/^translate3d\(0px, 0px, 0\) rotate\(0rad\) scaleX\(/);
    clock.runUntilIdle();
    await expect(done).resolves.toBe(true);
    expect(controller.state).toBe("idle");
    clock.timers.advance(200);
    expect(elements.dragPath.style.opacity).toBe("0");
  });

  it("interrupting a move resolves the first promise with false", async () => {
    const { controller, clock } = setup();
    controller.show();
    const first = controller.moveTo(500, 0);
    clock.frame();
    const second = controller.moveTo(0, 500);
    await expect(first).resolves.toBe(false);
    clock.runUntilIdle();
    await expect(second).resolves.toBe(true);
  });

  it("typing, waiting and hide are reflected on the root for CSS", () => {
    const { controller, root } = setup();
    controller.show();
    controller.typing(true);
    expect(root.getAttribute("data-state")).toBe("typing");
    controller.typing(false);
    controller.wait();
    expect(root.getAttribute("data-state")).toBe("waiting");
    controller.hide();
    expect(root.getAttribute("data-state")).toBe("hidden");
    controller.click();
    expect(controller.state).toBe("hidden");
  });

  it("moving while hidden only repositions silently", async () => {
    const { controller, clock } = setup();
    await expect(controller.moveTo(90, 90)).resolves.toBe(false);
    expect(controller.state).toBe("hidden");
    expect(clock.pending).toBe(0);
    expect(pos(controller)).toEqual({ x: 90, y: 90 });
  });

  it("dispose stops everything and ignores later calls", () => {
    const { controller, clock } = setup();
    controller.show();
    void controller.moveTo(100, 100);
    controller.dispose();
    expect(clock.pending).toBe(0);
    controller.click();
    void controller.moveTo(5, 5);
    expect(clock.pending).toBe(0);
  });
});

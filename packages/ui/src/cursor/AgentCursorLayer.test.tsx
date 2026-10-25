import { act, cleanup, render } from "@testing-library/react";
import { Profiler, createRef } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { AgentCursorLayer, type AgentCursorHandle } from "./AgentCursorLayer.js";
import { createFakeClock } from "./testClock.js";

afterEach(cleanup);

/** jsdom has no layout: give the layer a real box for the glides to cross. */
function withBox<T>(width: number, height: number, run: () => T): T {
  const w = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  const h = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
  try {
    return run();
  } finally {
    if (w) Object.defineProperty(HTMLElement.prototype, "clientWidth", w);
    if (h) Object.defineProperty(HTMLElement.prototype, "clientHeight", h);
  }
}

describe("AgentCursorLayer", () => {
  it("100 real actions cause 0 React re-renders (positions bypass React)", () => {
    const clock = createFakeClock();
    const ref = createRef<AgentCursorHandle>();
    let renders = 0;
    const { container } = withBox(800, 500, () =>
      render(
        <Profiler id="cursor" onRender={() => (renders += 1)}>
          <AgentCursorLayer ref={ref} effectsTier="full" reducedMotion={false} frameScheduler={clock} />
        </Profiler>,
      ),
    );
    const afterMount = renders;
    expect(afterMount).toBeGreaterThan(0);
    const cursor = ref.current;
    if (!cursor) throw new Error("no handle");
    act(() => {
      for (let i = 0; i < 100; i += 1) {
        cursor.moveTo({ x: (i % 10) / 10, y: (i % 7) / 7 });
        clock.frame();
      }
      clock.runUntilIdle();
    });
    expect(renders).toBe(afterMount);
    expect(cursor.isAnimating).toBe(false);
    expect(cursor.frameCount).toBeGreaterThanOrEqual(100);
    expect(container.querySelector(".pgc-layer")?.getAttribute("data-state")).toBe("moving");
  });

  it("is an inert overlay: aria-hidden, no pointer capture, hidden until a real action", () => {
    const ref = createRef<AgentCursorHandle>();
    const { container } = render(<AgentCursorLayer ref={ref} />);
    const layer = container.querySelector<HTMLElement>(".pgc-layer");
    expect(layer?.getAttribute("aria-hidden")).toBe("true");
    expect(layer?.getAttribute("data-state")).toBe("hidden");
    act(() => ref.current?.click({ x: 0.5, y: 0.5 }));
    expect(layer?.getAttribute("data-state")).toBe("clicking");
    act(() => ref.current?.stop());
    expect(layer?.getAttribute("data-state")).toBe("hidden");
  });

  it("maps through the guest frame size it is given, letterboxed in its box", () => {
    const ref = createRef<AgentCursorHandle>();
    const { container, rerender } = withBox(800, 800, () => render(<AgentCursorLayer ref={ref} frameSize={{ width: 1600, height: 900 }} />));
    expect(ref.current?.frameRectangle).toEqual({ x: 0, y: 175, width: 800, height: 450 });
    act(() => ref.current?.moveTo({ x: 0.5, y: 0 }));
    expect(container.querySelector<HTMLElement>(".pgc-pointer")?.style.transform).toBe("translate3d(400px, 175px, 0)");
    // A different guest resolution (a Windows guest at 1024x768) re-fits without remounting.
    rerender(<AgentCursorLayer ref={ref} frameSize={{ width: 1024, height: 768 }} />);
    expect(ref.current?.frameRectangle).toEqual({ x: 0, y: 100, width: 800, height: 600 });
    expect(container.querySelector<HTMLElement>(".pgc-pointer")?.style.transform).toBe("translate3d(400px, 100px, 0)");
  });

  it("tier and reduced motion reach the controller without remounting", () => {
    const ref = createRef<AgentCursorHandle>();
    const { container, rerender } = render(<AgentCursorLayer ref={ref} effectsTier="full" reducedMotion={false} />);
    const layer = container.querySelector(".pgc-layer");
    rerender(<AgentCursorLayer ref={ref} effectsTier="minimal" reducedMotion />);
    expect(container.querySelector(".pgc-layer")).toBe(layer);
    expect(layer?.getAttribute("data-tier")).toBe("minimal");
    expect(layer?.getAttribute("data-rm")).toBe("true");
  });

  it("disposes on unmount: the handle goes inert", () => {
    const ref = createRef<AgentCursorHandle>();
    const view = render(<AgentCursorLayer ref={ref} />);
    const handle = ref.current;
    view.unmount();
    expect(() => handle?.moveTo({ x: 0.5, y: 0.5 })).not.toThrow();
    expect(handle?.state).toBe("hidden");
  });
});

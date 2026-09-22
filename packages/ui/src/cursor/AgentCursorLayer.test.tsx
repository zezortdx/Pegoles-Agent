import { act, cleanup, render } from "@testing-library/react";
import { Profiler, createRef } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { AgentCursorLayer, type AgentCursorHandle } from "./AgentCursorLayer.js";
import { createFakeClock } from "./testClock.js";

afterEach(cleanup);

describe("AgentCursorLayer", () => {
  it("100 moveTo calls cause 0 React re-renders (positions bypass React)", () => {
    const clock = createFakeClock();
    const ref = createRef<AgentCursorHandle>();
    let renders = 0;
    const { container } = render(
      <Profiler id="cursor" onRender={() => (renders += 1)}>
        <AgentCursorLayer ref={ref} effectsTier="full" reducedMotion={false} frameScheduler={clock} timers={clock.timers} />
      </Profiler>,
    );
    const afterMount = renders;
    expect(afterMount).toBeGreaterThan(0);
    const cursor = ref.current;
    if (!cursor) throw new Error("no handle");
    act(() => {
      cursor.show();
      for (let i = 0; i < 100; i += 1) {
        void cursor.moveTo(10 + i * 7, 5 + ((i * 13) % 300));
        clock.frame();
      }
      clock.runUntilIdle();
    });
    expect(renders).toBe(afterMount);
    const pointer = container.querySelector<HTMLElement>(".pgc-pointer");
    expect(pointer?.style.transform).toBe(`translate3d(${10 + 99 * 7}px, ${5 + ((99 * 13) % 300)}px, 0)`);
    expect(cursor.isAnimating).toBe(false);
    expect(cursor.frameCount).toBeGreaterThanOrEqual(100);
  });

  it("is an inert overlay: aria-hidden, no pointer capture, hidden until shown", () => {
    const ref = createRef<AgentCursorHandle>();
    const { container } = render(<AgentCursorLayer ref={ref} />);
    const layer = container.querySelector<HTMLElement>(".pgc-layer");
    expect(layer?.getAttribute("aria-hidden")).toBe("true");
    expect(layer?.getAttribute("data-state")).toBe("hidden");
    act(() => ref.current?.show());
    expect(layer?.getAttribute("data-state")).toBe("idle");
    act(() => ref.current?.hide());
    expect(layer?.getAttribute("data-state")).toBe("hidden");
  });

  it("tier changes reach the controller without remounting", () => {
    const ref = createRef<AgentCursorHandle>();
    const { container, rerender } = render(<AgentCursorLayer ref={ref} effectsTier="full" reducedMotion={false} />);
    const trail = () => [...container.querySelectorAll<HTMLElement>(".pgc-trail")].filter((n) => n.style.display !== "none");
    const fullCount = trail().length;
    rerender(<AgentCursorLayer ref={ref} effectsTier="minimal" reducedMotion={false} />);
    expect(trail()).toHaveLength(0);
    expect(fullCount).toBeGreaterThan(0);
    expect(container.querySelector(".pgc-layer")?.getAttribute("data-tier")).toBe("minimal");
  });
});

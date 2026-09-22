import { act, cleanup, render } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AgentCursorOverlay,
  productionAgentCursorSource,
  type AgentCursorAction,
  type AgentCursorSource,
} from "./agentCursorSource.js";

afterEach(cleanup);

function fixtureSource(): AgentCursorSource & { emit(a: AgentCursorAction): void } {
  const listeners = new Set<(a: AgentCursorAction) => void>();
  return {
    id: "fixture",
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    emit(a) {
      for (const l of listeners) l(a);
    },
  };
}

describe("production AgentCursor source", () => {
  it("emits no positions without real agent events", () => {
    vi.useFakeTimers();
    try {
      const listener = vi.fn();
      const unsubscribe = productionAgentCursorSource.subscribe(listener);
      vi.advanceTimersByTime(60_000);
      expect(listener).not.toHaveBeenCalled();
      unsubscribe();
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the overlay unmounted (nothing rendered, nothing animating)", () => {
    vi.useFakeTimers();
    try {
      const { container } = render(<AgentCursorOverlay />);
      act(() => {
        vi.advanceTimersByTime(60_000);
      });
      expect(container.innerHTML).toBe("");
      expect(container.querySelector(".pgc-layer")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("AgentCursorOverlay with a source that has real events", () => {
  it("mounts on the first action, replays it, then routes actions without re-rendering", () => {
    const source = fixtureSource();
    let renders = 0;
    const { container } = render(
      <Profiler id="overlay" onRender={() => (renders += 1)}>
        <AgentCursorOverlay source={source} effectsTier="minimal" reducedMotion />
      </Profiler>,
    );
    expect(container.querySelector(".pgc-layer")).toBeNull();
    act(() => {
      source.emit({ kind: "show" });
      source.emit({ kind: "move", x: 40, y: 30 });
    });
    const layer = container.querySelector<HTMLElement>(".pgc-layer");
    expect(layer).not.toBeNull();
    expect(layer?.getAttribute("data-state")).toBe("idle");
    expect(container.querySelector<HTMLElement>(".pgc-pointer")?.style.transform).toBe("translate3d(40px, 30px, 0)");
    const settled = renders;
    act(() => {
      for (let i = 0; i < 50; i += 1) source.emit({ kind: "move", x: i, y: i });
      source.emit({ kind: "wait" });
    });
    expect(renders).toBe(settled);
    expect(layer?.getAttribute("data-state")).toBe("waiting");
  });
});

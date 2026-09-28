import { act, cleanup, render } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { AgentCursorOverlay, silentAgentCursorSource, type AgentCursorAction, type AgentCursorSource } from "./agentCursorSource.js";

afterEach(cleanup);

function fixtureSource(id = "fixture"): AgentCursorSource & { emit(a: AgentCursorAction): void; readonly listeners: number } {
  const listeners = new Set<(a: AgentCursorAction) => void>();
  return {
    id,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    emit(a) {
      for (const l of listeners) l(a);
    },
    get listeners() {
      return listeners.size;
    },
  };
}

const layerState = (container: HTMLElement) => container.querySelector(".pgc-layer")?.getAttribute("data-state");

describe("AgentCursorOverlay", () => {
  it("stays hidden with a silent source (no fake cursor)", () => {
    const { container } = render(<AgentCursorOverlay source={silentAgentCursorSource} />);
    expect(layerState(container)).toBe("hidden");
  });

  it("shows real actions without re-rendering React per action", () => {
    const source = fixtureSource();
    let renders = 0;
    const { container } = render(
      <Profiler id="overlay" onRender={() => (renders += 1)}>
        <AgentCursorOverlay source={source} />
      </Profiler>,
    );
    const mounted = renders;
    act(() => {
      source.emit({ kind: "move", at: { x: 0.2, y: 0.2 } });
      source.emit({ kind: "click", at: { x: 0.4, y: 0.4 }, count: 2 });
      source.emit({ kind: "drag", from: { x: 0.4, y: 0.4 }, to: { x: 0.6, y: 0.6 }, durationMs: 300 });
      source.emit({ kind: "scroll", at: { x: 0.5, y: 0.5 }, dx: 0, dy: 2 });
      source.emit({ kind: "typing", active: true });
    });
    expect(layerState(container)).toBe("typing");
    act(() => source.emit({ kind: "done" }));
    expect(layerState(container)).toBe("done");
    expect(renders).toBe(mounted);
  });

  it("Stop hides it at once; unmounting unsubscribes", () => {
    const source = fixtureSource();
    const view = render(<AgentCursorOverlay source={source} />);
    act(() => source.emit({ kind: "click", at: { x: 0.5, y: 0.5 }, count: 1 }));
    act(() => source.emit({ kind: "stop" }));
    expect(layerState(view.container)).toBe("hidden");
    expect(source.listeners).toBe(1);
    view.unmount();
    expect(source.listeners).toBe(0);
  });

  it("a replaced computer starts clean: the old source is dropped, nothing carries over", () => {
    const first = fixtureSource("vm-1");
    const second = fixtureSource("vm-2");
    const view = render(<AgentCursorOverlay key="vm-1" source={first} />);
    act(() => first.emit({ kind: "move", at: { x: 0.9, y: 0.9 } }));
    expect(layerState(view.container)).toBe("moving");
    view.rerender(<AgentCursorOverlay key="vm-2" source={second} />);
    expect(first.listeners).toBe(0);
    expect(layerState(view.container)).toBe("hidden");
    // A late event from the old computer reaches nobody.
    act(() => first.emit({ kind: "click", at: { x: 0.1, y: 0.1 }, count: 1 }));
    expect(layerState(view.container)).toBe("hidden");
    act(() => second.emit({ kind: "move", at: { x: 0.3, y: 0.3 } }));
    expect(layerState(view.container)).toBe("moving");
  });

  it("actions from before it mounted are never replayed", () => {
    const source = fixtureSource();
    source.emit({ kind: "click", at: { x: 0.5, y: 0.5 }, count: 1 });
    const { container } = render(<AgentCursorOverlay source={source} />);
    expect(layerState(container)).toBe("hidden");
  });
});

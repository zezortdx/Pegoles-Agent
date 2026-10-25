import { useLayoutEffect, useRef } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { FluxGlassRoot, resetOffscreenObserverForTests } from "@pegoles/ui";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PresenceAvatar } from "./PresenceAvatar";
import { taskActivity } from "../state/agentState";
import type { AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";

let hidden = false;
let reduced = false;
let nextFrame = 0;
const frames = new Map<number, FrameRequestCallback>();
const mediaListeners = new Set<() => void>();
beforeEach(() => {
  hidden = false;
  reduced = false;
  frames.clear();
  mediaListeners.clear();
  resetOffscreenObserverForTests();
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => hidden ? "hidden" : "visible");
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => { frames.delete(id); }));
  vi.stubGlobal("matchMedia", vi.fn(() => ({
    get matches() { return reduced; },
    addEventListener: (_event: string, listener: () => void) => { mediaListeners.add(listener); },
    removeEventListener: (_event: string, listener: () => void) => { mediaListeners.delete(listener); },
  })));
});
afterEach(() => {
  cleanup();
  resetOffscreenObserverForTests();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function tick(time: number) {
  act(() => {
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach((callback) => callback(time));
  });
}
function svg(container: HTMLElement) { return container.querySelector("svg.presence-avatar") as SVGSVGElement; }

describe("PresenceAvatar", () => {
  it("renders a named SVG in the exact layout size using silver tokens", () => {
    const { container } = render(<PresenceAvatar mode="thinking" size={52} />);
    const avatar = screen.getByRole("img", { name: "Pegoles is thinking" });
    expect(avatar.tagName.toLowerCase()).toBe("svg");
    expect(avatar.getAttribute("width")).toBe("52");
    expect(avatar.getAttribute("height")).toBe("52");
    expect(avatar.getAttribute("data-mode")).toBe("thinking");
    expect(avatar.querySelector("linearGradient stop")?.getAttribute("stop-color")).toContain("--presence-white");
    expect(container.querySelector("img, canvas, foreignObject")).toBeNull();
  });

  it("uses one static pose under reduced motion, while still responding to state changes", () => {
    reduced = true;
    const view = render(<PresenceAvatar mode="using-computer" size={64} />);
    const avatar = svg(view.container);
    const first = avatar.innerHTML;
    expect(avatar.getAttribute("data-frame")).toBe("static");
    expect(frames.size).toBe(0);
    tick(1000);
    view.rerender(<PresenceAvatar mode="using-computer" size={64} pulse={1} nudge={1} reaction={1} />);
    expect(avatar.innerHTML).toBe(first);
    expect(frames.size).toBe(0);
    view.rerender(<PresenceAvatar mode="needs-user" size={64} />);
    expect(avatar.innerHTML).not.toBe(first);
    expect(avatar.getAttribute("aria-label")).toBe("Pegoles needs you");
    expect(avatar.querySelector(".presence-avatar__notification")).not.toBeNull();
  });

  it("honors the app's reduced-motion override and minimal effects tier", () => {
    const view = render(<FluxGlassRoot tier="full" reducedMotion><PresenceAvatar mode="working" size={64} /></FluxGlassRoot>);
    expect(svg(view.container).getAttribute("data-frame")).toBe("static");
    expect(frames.size).toBe(0);
    view.rerender(<FluxGlassRoot tier="minimal"><PresenceAvatar mode="working" size={64} /></FluxGlassRoot>);
    expect(svg(view.container).getAttribute("data-frame")).toBe("static");
    expect(frames.size).toBe(0);
  });

  it("stops and restarts when the OS motion preference changes", () => {
    const view = render(<PresenceAvatar mode="working" size={64} />);
    expect(frames.size).toBe(1);
    act(() => { reduced = true; mediaListeners.forEach((listener) => listener()); });
    expect(svg(view.container).getAttribute("data-frame")).toBe("static");
    expect(frames.size).toBe(0);
    act(() => { reduced = false; mediaListeners.forEach((listener) => listener()); });
    expect(svg(view.container).getAttribute("data-frame")).toBe("live");
    expect(frames.size).toBe(1);
  });

  it("renders the new Core state before passive effects, without a stale orbit", () => {
    reduced = true;
    const observed: number[] = [];
    function Snapshot({ mode }: { mode: "using-computer" | "offline" }) {
      const root = useRef<HTMLDivElement>(null);
      useLayoutEffect(() => {
        observed.push(root.current?.querySelectorAll(".presence-avatar__orbit").length ?? -1);
      }, [mode]);
      return <div ref={root}><PresenceAvatar mode={mode} size={64} /></div>;
    }
    const view = render(<Snapshot mode="using-computer" />);
    expect(observed[0]).toBeGreaterThan(0);
    view.rerender(<Snapshot mode="offline" />);
    expect(observed[observed.length - 1]).toBe(0);
  });

  it("pauses an offscreen avatar and resumes when it returns", () => {
    let notify: IntersectionObserverCallback = () => undefined;
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { notify = callback; }
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    });
    const view = render(<PresenceAvatar mode="working" size={64} />);
    const avatar = svg(view.container);
    expect(frames.size).toBe(1);
    const rect = avatar.getBoundingClientRect();
    const entry = (isIntersecting: boolean): IntersectionObserverEntry => ({
      target: avatar, isIntersecting, boundingClientRect: rect, intersectionRatio: isIntersecting ? 1 : 0,
      intersectionRect: rect, rootBounds: null, time: 0,
    });
    act(() => notify([entry(false)], {} as IntersectionObserver));
    expect(frames.size).toBe(0);
    act(() => notify([entry(true)], {} as IntersectionObserver));
    expect(frames.size).toBe(1);
  });

  it("limits idle updates to 30 fps", () => {
    const view = render(<PresenceAvatar mode="idle" size={152} />);
    const first = svg(view.container).innerHTML;
    tick(10);
    tick(20);
    expect(svg(view.container).innerHTML).toBe(first);
    tick(60);
    expect(svg(view.container).innerHTML).not.toBe(first);
  });

  it("applies a real gaze target without moving a reduced-motion pose", () => {
    const view = render(<PresenceAvatar mode="idle" size={64} look={{ x: 1, y: -1 }} />);
    const first = svg(view.container).querySelector(".presence-avatar__eye")?.getAttribute("transform");
    tick(10);
    tick(60);
    tick(110);
    expect(svg(view.container).querySelector(".presence-avatar__eye")?.getAttribute("transform")).not.toBe(first);
    expect(svg(view.container).innerHTML).not.toMatch(/NaN|Infinity/);
    act(() => { reduced = true; mediaListeners.forEach((listener) => listener()); });
    const still = svg(view.container).innerHTML;
    view.rerender(<PresenceAvatar mode="idle" size={64} look={{ x: -1, y: 1 }} />);
    expect(svg(view.container).innerHTML).toBe(still);
  });

  it("shares the animation loop and cancels it on unmount", () => {
    const view = render(<><PresenceAvatar mode="idle" size={64} /><PresenceAvatar mode="thinking" size={20} /></>);
    expect(frames.size).toBe(1);
    view.unmount();
    expect(frames.size).toBe(0);
  });

  it("does not update geometry while the tab is hidden and resumes without a jump", () => {
    const view = render(<PresenceAvatar mode="using-computer" size={64} />);
    tick(10);
    tick(50);
    const before = svg(view.container).innerHTML;
    act(() => { hidden = true; document.dispatchEvent(new Event("visibilitychange")); });
    expect(frames.size).toBe(0);
    tick(50_000);
    expect(svg(view.container).innerHTML).toBe(before);
    act(() => { hidden = false; document.dispatchEvent(new Event("visibilitychange")); });
    tick(60_000);
    expect(svg(view.container).innerHTML).toBe(before);
    tick(60_040);
    expect(svg(view.container).innerHTML).not.toBe(before);
  });

  it.each([20, 40, 52, 55])("uses thinking instead of an orbit from the first frame at %i px", (size) => {
    const view = render(<PresenceAvatar mode="using-computer" size={size} />);
    const avatar = svg(view.container);
    expect(avatar.getAttribute("data-engine-state")).toBe("thinking");
    expect(avatar.querySelectorAll(".presence-avatar__orbit")).toHaveLength(0);
    tick(10);
    tick(60);
    expect(avatar.getAttribute("data-engine-state")).toBe("thinking");
  });

  it("shows orbit geometry at 56 px, and drops it on resize", () => {
    reduced = true;
    const view = render(<PresenceAvatar mode="using-computer" size={56} />);
    expect(svg(view.container).querySelectorAll(".presence-avatar__orbit").length).toBeGreaterThan(0);
    view.rerender(<PresenceAvatar mode="using-computer" size={55} />);
    expect(svg(view.container).getAttribute("data-engine-state")).toBe("thinking");
    expect(svg(view.container).querySelectorAll(".presence-avatar__orbit")).toHaveLength(0);
  });

  it("pauses idle life on blur but keeps real work moving until hidden", () => {
    const view = render(<FluxGlassRoot tier="full"><PresenceAvatar mode="idle" size={64} /></FluxGlassRoot>);
    expect(frames.size).toBe(1);
    fireEvent.blur(window);
    expect(frames.size).toBe(0);
    view.rerender(<FluxGlassRoot tier="full" busy><PresenceAvatar mode="working" size={64} /></FluxGlassRoot>);
    expect(frames.size).toBe(1);
    view.rerender(<FluxGlassRoot tier="full" busy windowHidden><PresenceAvatar mode="working" size={64} /></FluxGlassRoot>);
    expect(frames.size).toBe(0);
  });

  it("freezes offline, blocked and error poses without scheduling frames", () => {
    const view = render(<PresenceAvatar mode="offline" size={64} />);
    expect(frames.size).toBe(0);
    for (const mode of ["blocked", "error"] as const) {
      view.rerender(<PresenceAvatar mode={mode} size={64} />);
      expect(svg(view.container).getAttribute("data-engine-state")).toBe("exclaim");
      expect(frames.size).toBe(0);
    }
  });

  it("follows actual Core action, approval, failure and connection facts", () => {
    const at = "2026-10-01T00:00:00Z";
    const task: AgentTask = { id: "task", title: "Read the document", status: "running", created_at: at, updated_at: at };
    const status = { agent_busy: true, control_owner: "agent", model: "configured" } as StatusPayload;
    const started: AgentEvent = { type: "action_started", request: { action_id: "action", task_id: "task", computer_id: "computer", action: { type: "click" }, requested_at: at } };
    const inputs = { task, status, events: [started], connected: true };
    const view = render(<PresenceAvatar mode={taskActivity(inputs).mode} size={64} />);
    expect(svg(view.container).getAttribute("data-engine-state")).toBe("orbit");
    view.rerender(<PresenceAvatar mode={taskActivity({ ...inputs, status: { ...status, agent_busy: false } }).mode} size={64} />);
    expect(svg(view.container).getAttribute("data-engine-state")).toBe("thinking");
    view.rerender(<PresenceAvatar mode={taskActivity({ ...inputs, task: { ...task, status: "waiting_for_approval" } }).mode} size={64} />);
    expect(svg(view.container).getAttribute("data-engine-state")).toBe("notify");
    view.rerender(<PresenceAvatar mode={taskActivity({ ...inputs, task: { ...task, status: "failed" } }).mode} size={64} />);
    expect(svg(view.container).getAttribute("data-engine-state")).toBe("exclaim");
    view.rerender(<PresenceAvatar mode={taskActivity({ ...inputs, connected: false }).mode} size={64} />);
    expect(svg(view.container).getAttribute("aria-label")).toBe("Pegoles is offline");
  });
});

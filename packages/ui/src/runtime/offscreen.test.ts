import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { observedOffscreenCount, observeOffscreen, resetOffscreenObserverForTests } from "./offscreen.js";

type Callback = (entries: IntersectionObserverEntry[]) => void;

class FakeIntersectionObserver {
  static instances: FakeIntersectionObserver[] = [];
  readonly observed = new Set<Element>();
  constructor(readonly callback: Callback) {
    FakeIntersectionObserver.instances.push(this);
  }
  observe(el: Element): void {
    this.observed.add(el);
  }
  unobserve(el: Element): void {
    this.observed.delete(el);
  }
  disconnect(): void {
    this.observed.clear();
  }
  fire(target: Element, isIntersecting: boolean): void {
    this.callback([{ target, isIntersecting } as IntersectionObserverEntry]);
  }
}

describe("shared offscreen observer", () => {
  beforeEach(() => {
    FakeIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    resetOffscreenObserverForTests();
  });

  afterEach(() => {
    resetOffscreenObserverForTests();
    vi.unstubAllGlobals();
  });

  it("uses ONE observer for every element and writes data-offscreen", () => {
    const a = document.createElement("div");
    const b = document.createElement("div");
    const stopA = observeOffscreen(a);
    const stopB = observeOffscreen(b);
    expect(FakeIntersectionObserver.instances).toHaveLength(1);
    const io = FakeIntersectionObserver.instances[0];
    if (!io) throw new Error("no observer");
    io.fire(a, false);
    io.fire(b, true);
    expect(a.getAttribute("data-offscreen")).toBe("true");
    expect(b.getAttribute("data-offscreen")).toBe("false");
    stopA();
    stopB();
    expect(observedOffscreenCount()).toBe(0);
    expect(a.hasAttribute("data-offscreen")).toBe(false);
  });

  it("notifies listeners and keeps observing until the last listener leaves", () => {
    const el = document.createElement("div");
    const first = vi.fn();
    const second = vi.fn();
    const stopFirst = observeOffscreen(el, first);
    const stopSecond = observeOffscreen(el, second);
    const io = FakeIntersectionObserver.instances[0];
    if (!io) throw new Error("no observer");
    io.fire(el, false);
    expect(first).toHaveBeenCalledWith(true);
    expect(second).toHaveBeenCalledWith(true);
    stopFirst();
    expect(io.observed.has(el)).toBe(true);
    stopSecond();
    expect(io.observed.has(el)).toBe(false);
  });

  it("degrades to 'always onscreen' without IntersectionObserver", () => {
    vi.stubGlobal("IntersectionObserver", undefined);
    resetOffscreenObserverForTests();
    const el = document.createElement("div");
    const stop = observeOffscreen(el);
    expect(el.hasAttribute("data-offscreen")).toBe(false);
    stop();
  });
});

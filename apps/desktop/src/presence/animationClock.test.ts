import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { subscribeAvatarClock } from "./animationClock";

let hidden = false;
let nextFrame = 0;
const frames = new Map<number, FrameRequestCallback>();
const cleanups: (() => void)[] = [];

beforeEach(() => {
  hidden = false;
  frames.clear();
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => { frames.delete(id); }));
});
afterEach(() => {
  cleanups.splice(0).forEach((stop) => stop());
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function tick(time: number) {
  const pending = [...frames.values()];
  frames.clear();
  pending.forEach((callback) => callback(time));
}
function subscribe(callback: (dt: number) => void) {
  const stop = subscribeAvatarClock(callback);
  cleanups.push(stop);
  return stop;
}

describe("shared avatar clock", () => {
  it("uses one frame loop and releases it after the last subscriber", () => {
    const a = vi.fn();
    const b = vi.fn();
    const stopA = subscribe(a);
    const stopB = subscribe(b);
    expect(frames.size).toBe(1);
    tick(0);
    expect(a).toHaveBeenLastCalledWith(0);
    expect(b).toHaveBeenLastCalledWith(0);
    stopA();
    tick(20);
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenLastCalledWith(0.02);
    stopB();
    expect(frames.size).toBe(0);
    stopB();
    expect(frames.size).toBe(0);
  });

  it("caps long frame gaps at 64 ms", () => {
    const sample = vi.fn();
    subscribe(sample);
    tick(10);
    tick(5010);
    expect(sample).toHaveBeenLastCalledWith(0.064);
  });

  it("stops while hidden and resumes without advancing hidden time", () => {
    const sample = vi.fn();
    subscribe(sample);
    tick(100);
    hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(frames.size).toBe(0);
    hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(frames.size).toBe(1);
    tick(90_000);
    expect(sample).toHaveBeenLastCalledWith(0);
    tick(90_020);
    expect(sample).toHaveBeenLastCalledWith(0.02);
  });

  it("does not schedule frames when mounted into a hidden document", () => {
    hidden = true;
    subscribe(vi.fn());
    expect(frames.size).toBe(0);
    hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(frames.size).toBe(1);
  });
});

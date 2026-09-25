import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FluxGlassRoot } from "@pegoles/ui";
import { MEASURE_MAX_WAIT_MS, NativeSlot } from "./NativeSlot";
import { api } from "../lib/tauri";

let top = 100;
let observed: (() => void) | null = null;
beforeEach(() => {
  top = 100;
  observed = null;
  vi.useFakeTimers();
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { observed = callback; } observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({ x: 20, y: top, width: 640, height: 400, top, left: 20, bottom: top + 400, right: 660, toJSON() {} }));
  vi.spyOn(api, "setDisplayGeometry").mockResolvedValue(undefined);
  vi.spyOn(api, "detachDisplay").mockResolvedValue(undefined);
});
afterEach(async () => {
  await act(async () => { cleanup(); await vi.runAllTimersAsync(); });
  vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
});
const onError = vi.fn();
/** Past the max-wait deadline, whether or not frames run. */
async function settle() { await act(async () => { await vi.advanceTimersByTimeAsync(MEASURE_MAX_WAIT_MS + 10); }); }

describe("native display geometry", () => {
  it("updates a moved slot but suppresses unchanged geometry and preserves attachment", async () => {
    const view = render(<NativeSlot enabled onError={onError} />);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenCalledOnce();
    view.rerender(<NativeSlot enabled onError={onError} />);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenCalledOnce();
    top = 180;
    view.rerender(<NativeSlot enabled onError={onError} display={{ width_px: 1440, height_px: 900 }} />);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ rect: expect.objectContaining({ y: 180 }) }));
    expect(api.detachDisplay).not.toHaveBeenCalled();
  });

  it("coalesces a burst of layout changes into one native update", async () => {
    render(<NativeSlot enabled onError={onError} />);
    await settle();
    vi.mocked(api.setDisplayGeometry).mockClear();
    for (const y of [120, 140, 160, 200]) { top = y; observed?.(); }
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenCalledOnce();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ rect: expect.objectContaining({ y: 200 }) }));
  });

  it("never asks the native view to animate, whatever the motion preference", async () => {
    const view = render(<FluxGlassRoot tier="full"><NativeSlot enabled onError={onError} /></FluxGlassRoot>);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ animate_ms: 0 }));
    top = 220;
    view.rerender(<FluxGlassRoot tier="minimal" reducedMotion><NativeSlot enabled onError={onError} /></FluxGlassRoot>);
    await settle();
    expect(api.detachDisplay).not.toHaveBeenCalled();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ animate_ms: 0 }));
  });

  it("hides the view while obscured and shows it again without detaching", async () => {
    const view = render(<NativeSlot enabled obscured onError={onError} />);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    view.rerender(<NativeSlot enabled onError={onError} />);
    await settle();
    expect(api.setDisplayGeometry).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
    expect(api.detachDisplay).not.toHaveBeenCalled();
  });

  it("detaches when the Computer leaves the workspace", async () => {
    const view = render(<NativeSlot enabled onError={onError} />);
    await settle();
    await act(async () => view.unmount());
    expect(api.detachDisplay).toHaveBeenCalledOnce();
  });
});

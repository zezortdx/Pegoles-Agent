import { describe, expect, it, vi } from "vitest";
import { overlayPoint } from "./agentCursorFeed";

describe("overlayPoint (real slot geometry)", () => {
  it("maps normalized positions through the live slot rect", () => {
    document.body.innerHTML = '<div class="computer__screen"><div data-framebuffer-slot=""></div></div>';
    const wrap = document.querySelector(".computer__screen") as HTMLElement;
    const slot = document.querySelector("[data-framebuffer-slot]") as HTMLElement;
    vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue({
      left: 100, top: 50, width: 800, height: 600, right: 900, bottom: 650, x: 100, y: 50, toJSON: () => undefined,
    } as DOMRect);
    vi.spyOn(slot, "getBoundingClientRect").mockReturnValue({
      left: 100, top: 100, width: 720, height: 450, right: 820, bottom: 550, x: 100, y: 100, toJSON: () => undefined,
    } as DOMRect);
    expect(overlayPoint(0.5, 0.5)).toEqual({ x: 360, y: 275 });
    expect(overlayPoint(0, 0)).toEqual({ x: 0, y: 50 });
    document.body.innerHTML = "";
    expect(overlayPoint(0.5, 0.5)).toBeNull();
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FluxGlassRoot } from "@pegoles/ui";
import { PegolesPresence, pointerPose } from "./PegolesPresence";

afterEach(cleanup);

describe("PegolesPresence", () => {
  it("renders the animated SVG and names the real state", () => {
    render(<FluxGlassRoot tier="full"><PegolesPresence mode="thinking" size={152} /></FluxGlassRoot>);
    const presence = screen.getByRole("img", { name: "Pegoles is thinking" });
    expect(presence.getAttribute("data-mode")).toBe("thinking");
    const avatar = presence.querySelector("svg.presence-avatar");
    expect(avatar?.getAttribute("data-engine-state")).toBe("thinking");
    expect(avatar?.querySelector(".presence-avatar__body")).toBeTruthy();
    expect(presence.querySelector("img")).toBeNull();
    expect(presence.querySelector("canvas")).toBeNull();
  });

  it("uses the SVG renderer for small marks", () => {
    const { container } = render(<PegolesPresence mode="idle" size={36} decorative />);
    expect(container.querySelector("svg.presence-avatar")?.getAttribute("width")).toBe("36");
  });

  it("keeps an exact layout box and accepts a custom label", () => {
    render(<PegolesPresence mode="needs-user" size={40} label="Pegoles needs your approval" />);
    const presence = screen.getByRole("img", { name: "Pegoles needs your approval" });
    expect(presence.style.getPropertyValue("--presence-size")).toBe("40px");
  });

  it("is hidden from assistive technology when decorative", () => {
    const { container } = render(<PegolesPresence mode="idle" size={18} decorative field={false} />);
    const presence = container.querySelector(".presence");
    expect(presence?.getAttribute("aria-hidden")).toBe("true");
    expect(presence?.getAttribute("role")).toBeNull();
    expect(presence?.querySelector(".presence__field")).toBeNull();
  });

  it("ripples on new events only, not on the count it mounted with", () => {
    const view = render(<PegolesPresence mode="working" size={64} pulse={4} />);
    expect(view.container.querySelector(".presence-avatar__pulse")).toBeNull();
    view.rerender(<PegolesPresence mode="working" size={64} pulse={5} />);
    expect(view.container.querySelector(".presence-avatar__pulse")).toBeTruthy();
  });

  it("preserves the real mode while using a legible engine pose at small sizes", () => {
    const view = render(<PegolesPresence mode="using-computer" size={40} />);
    expect(view.container.querySelector(".presence")?.getAttribute("data-mode")).toBe("using-computer");
    expect(view.container.querySelector(".presence-avatar")?.getAttribute("data-engine-state")).toBe("thinking");
    view.rerender(<PegolesPresence mode="waiting" size={40} />);
    expect(view.container.querySelector(".presence-avatar")?.getAttribute("data-engine-state")).toBe("notify");
  });

  it("is a real button when pressable: keyboard and pointer both press", () => {
    const onPress = vi.fn();
    render(<PegolesPresence mode="idle" size={152} pressable onPress={onPress} />);
    const button = screen.getByRole("button", { name: "Pegoles is idle" });
    expect(button.tabIndex).toBe(0);
    fireEvent.keyDown(button, { key: "Enter" });
    fireEvent.keyDown(button, { key: " " });
    fireEvent.keyDown(button, { key: "Enter", repeat: true });
    fireEvent.keyDown(button, { key: "a" });
    fireEvent.click(button);
    expect(onPress).toHaveBeenCalledTimes(3);
  });

  it("reacts to a new keystroke through the avatar controller", () => {
    const view = render(<PegolesPresence mode="idle" size={152} nudge={0} />);
    const root = view.container.querySelector<HTMLElement>(".presence");
    view.rerender(<PegolesPresence mode="idle" size={152} nudge={1} />);
    expect(root?.querySelector(".presence-avatar")?.getAttribute("data-engine-state")).toBe("wide");
  });

  it("renders a stopped pose for a safety block", () => {
    const { container } = render(<PegolesPresence mode="blocked" size={64} />);
    const root = container.querySelector<HTMLElement>(".presence");
    expect(root?.querySelector(".presence-avatar")?.getAttribute("data-engine-state")).toBe("exclaim");
  });
});

describe("pointer pose", () => {
  it("tracks up to 8% of the mark and tilts up to 7°, easing with distance", () => {
    const close = pointerPose(260, 0);
    expect(close.eyeX).toBeCloseTo(0.08);
    expect(close.yaw).toBeGreaterThan(0);
    expect(close.yaw).toBeLessThanOrEqual(7);
    const far = pointerPose(2000, 0);
    expect(far.eyeX).toBeCloseTo(0.08 * 0.35);
    expect(far.yaw).toBe(0);
    const below = pointerPose(0, 100);
    expect(below.eyeY).toBeGreaterThan(0);
    expect(below.pitch).toBeGreaterThan(0);
    expect(pointerPose(0, 0)).toEqual({ eyeX: 0, eyeY: 0, pitch: 0, yaw: 0 });
  });
});

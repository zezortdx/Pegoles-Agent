import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FluxGlassRoot } from "@pegoles/ui";
import { PegolesPresence, pointerPose } from "./PegolesPresence";
import { PresenceQualityProvider } from "./quality";

afterEach(cleanup);

describe("PegolesPresence", () => {
  it("paints the SVG renderer without WebGL and names the state", () => {
    render(<FluxGlassRoot tier="full"><PresenceQualityProvider quality="full"><PegolesPresence mode="thinking" size={152} /></PresenceQualityProvider></FluxGlassRoot>);
    const presence = screen.getByRole("img", { name: "Pegoles is thinking" });
    expect(presence.getAttribute("data-mode")).toBe("thinking");
    expect(presence.getAttribute("data-renderer")).toBe("svg");
    expect(presence.querySelector(".presence__svg")).toBeTruthy();
    expect(presence.querySelector("canvas")).toBeNull();
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
    expect(view.container.querySelector(".presence__pulse")).toBeNull();
    view.rerender(<PegolesPresence mode="working" size={64} pulse={5} />);
    expect(view.container.querySelector(".presence__pulse")).toBeTruthy();
  });

  it("gates continuous loops by kind: real work versus decorative life", () => {
    const view = render(<PegolesPresence mode="working" size={64} />);
    expect(view.container.querySelector(".presence__object")?.classList.contains("pg-work-anim")).toBe(true);
    view.rerender(<PegolesPresence mode="idle" size={64} />);
    expect(view.container.querySelector(".presence__object")?.classList.contains("pg-ambient")).toBe(true);
    view.rerender(<PegolesPresence mode="done" size={64} />);
    const object = view.container.querySelector(".presence__object");
    expect(object?.classList.contains("pg-ambient") || object?.classList.contains("pg-work-anim")).toBe(false);
  });

  it("is a real button when pressable: keyboard and pointer both press", () => {
    const onPress = vi.fn();
    render(<PegolesPresence mode="idle" size={152} pressable onPress={onPress} />);
    const button = screen.getByRole("button", { name: "Pegoles" });
    expect(button.tabIndex).toBe(0);
    fireEvent.keyDown(button, { key: "Enter" });
    fireEvent.keyDown(button, { key: " " });
    fireEvent.keyDown(button, { key: "Enter", repeat: true });
    fireEvent.keyDown(button, { key: "a" });
    fireEvent.click(button);
    expect(onPress).toHaveBeenCalledTimes(3);
  });

  it("reacts to keystrokes without re-rendering, throttled", () => {
    const view = render(<PegolesPresence mode="attentive" size={152} nudge={0} />);
    const root = view.container.querySelector<HTMLElement>(".presence");
    view.rerender(<PegolesPresence mode="attentive" size={152} nudge={1} />);
    expect(root?.style.getPropertyValue("--p-kick")).toBe("1");
  });

  it("writes the pose from the targets table", () => {
    const { container } = render(<PegolesPresence mode="blocked" size={64} />);
    const root = container.querySelector<HTMLElement>(".presence");
    expect(root?.style.getPropertyValue("--p-lid")).toBe("0.46");
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

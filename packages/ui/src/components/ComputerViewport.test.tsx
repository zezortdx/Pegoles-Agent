import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ComputerViewport } from "./ComputerViewport.js";
import { VIEWPORT_STATE_SPECS, VIEWPORT_STATES, type BootStage } from "./viewportModel.js";

const STAGES: BootStage[] = [
  { id: "vm", label: "Virtual machine", status: "done", detail: "1.1 s" },
  { id: "guest", label: "Guest runtime", status: "active" },
  { id: "session", label: "Graphical session", status: "pending" },
  { id: "display", label: "Display", status: "pending" },
];

describe("ComputerViewport", () => {
  it.each(VIEWPORT_STATES)("state %s: label, tone and polite announcement", (state) => {
    const spec = VIEWPORT_STATE_SPECS[state];
    const { container } = render(<ComputerViewport state={state} bootStages={spec.booting ? STAGES : []} />);
    const region = screen.getByRole("region", { name: "Pegoles Computer" });
    expect(region.getAttribute("data-state")).toBe(state);
    expect(region.getAttribute("data-energy")).toBe(spec.energy);
    const live = container.querySelector("[aria-live='polite'][role='status']");
    expect(live?.textContent).toBe(spec.announcement);
    const pill = container.querySelector(".pg-viewport__status");
    expect(pill?.textContent).toContain(spec.label);
    expect(pill?.getAttribute("data-tone")).toBe(spec.tone);
    // Never a percentage without a real measure.
    expect(region.textContent).not.toMatch(/\d+\s?%/);
  });

  it("keeps the display aspect ratio on the frame and reports the slot element", () => {
    const slotRef = vi.fn();
    const { container } = render(
      <ComputerViewport state="ready" display={{ width_px: 1280, height_px: 800 }} slotRef={slotRef} />,
    );
    const frame = container.querySelector<HTMLElement>(".pg-viewport__frame");
    expect(frame?.style.aspectRatio).toBe("1280 / 800");
    const slot = container.querySelector("[data-framebuffer-slot]");
    expect(slotRef).toHaveBeenCalledWith(slot);
    expect(container.textContent).toContain("1280 × 800");
  });

  it("renders NOTHING inside the slot when a native surface is present", () => {
    for (const state of VIEWPORT_STATES) {
      const { container, unmount } = render(
        <ComputerViewport
          state={state}
          nativeSurface
          bootStages={STAGES}
          errorMessage="The display stopped responding."
          placeholder={<span>placeholder</span>}
        />,
      );
      const slot = container.querySelector("[data-framebuffer-slot]");
      expect(slot?.childNodes.length, state).toBe(0);
      unmount();
    }
  });

  it("shows real boot stages with indeterminate progress when no percentage exists", () => {
    render(<ComputerViewport state="guest_connecting" bootStages={STAGES} />);
    const list = screen.getByRole("list", { name: "Startup stages" });
    const items = within(list).getAllByRole("listitem");
    expect(items.map((i) => i.getAttribute("data-status"))).toEqual(["done", "active", "pending", "pending"]);
    expect(items[1]?.textContent).toContain("In progress");
    const bar = screen.getByRole("progressbar", { name: "Guest runtime" });
    expect(bar.hasAttribute("aria-valuenow")).toBe(false);
    expect(list.textContent).not.toContain("%");
  });

  it("shows a percentage only for a stage with a real measure", () => {
    render(
      <ComputerViewport
        state="preparing"
        bootStages={[{ id: "image", label: "Pegoles Base Image", status: "active", progress: 0.37, detail: "412 MB of 1.1 GB" }]}
      />,
    );
    expect(screen.getByRole("progressbar", { name: "Pegoles Base Image" }).getAttribute("aria-valuenow")).toBe("37");
    expect(screen.getByText("37%")).toBeTruthy();
  });

  it("offers Take control only when the state allows and a handler exists", () => {
    const onTakeControl = vi.fn();
    const { rerender } = render(<ComputerViewport state="ready" onTakeControl={onTakeControl} />);
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));
    expect(onTakeControl).toHaveBeenCalledTimes(1);
    rerender(<ComputerViewport state="starting" onTakeControl={onTakeControl} />);
    expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
    rerender(<ComputerViewport state="ready" />);
    expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
  });

  it("while the user controls: clear statement, shortcut and Return to Pegoles", () => {
    const onReturnControl = vi.fn();
    const ref = { current: null as HTMLButtonElement | null };
    const { container } = render(
      <ComputerViewport state="user_controlled" onReturnControl={onReturnControl} controlButtonRef={ref} />,
    );
    expect(container.textContent).toContain("You're controlling Pegoles Computer");
    expect(container.querySelector("kbd")?.textContent).toBe("⌃⌥⎋");
    const button = screen.getByRole("button", { name: "Return to Pegoles" });
    expect(ref.current).toBe(button);
    button.focus();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    expect(onReturnControl).toHaveBeenCalledTimes(1);
  });

  it("announces errors assertively in the slot, or in the footer over a native surface", () => {
    const { rerender } = render(<ComputerViewport state="error" errorMessage="Virtualization failed to start." />);
    expect(screen.getByRole("alert").textContent).toContain("Virtualization failed to start.");
    rerender(<ComputerViewport state="error" nativeSurface errorMessage="The display stopped responding." />);
    expect(screen.getByRole("alert").textContent).toContain("The display stopped responding.");
  });

  it("compact variant surfaces only the current stage", () => {
    render(<ComputerViewport state="guest_connecting" bootStages={STAGES} variant="compact" />);
    expect(screen.queryByRole("list", { name: "Startup stages" })).toBeNull();
    expect(screen.getByText("Guest runtime")).toBeTruthy();
  });

  it("agent_active gets the ambient breathing edge; other states do not", () => {
    const { container, rerender } = render(<ComputerViewport state="agent_active" />);
    expect(container.querySelector("[data-layer='breath']")?.classList.contains("pg-ambient")).toBe(true);
    rerender(<ComputerViewport state="ready" />);
    expect(container.querySelector("[data-layer='breath']")).toBeNull();
  });
});

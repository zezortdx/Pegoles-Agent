import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ComputerPanel, type ComputerPanelProps } from "./ComputerPanel";
import { computerModel } from "../state/computerModel";
import { api, type StatusPayload } from "../lib/tauri";
import type { Snapshot } from "./useScreenSnapshot";

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({ x: 0, y: 0, width: 400, height: 250, top: 0, left: 0, bottom: 250, right: 400, toJSON() {} }));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const status: StatusPayload = {
  core: "running", model: "not_configured", backend: "real", computer_created: true, computer_state: "running", computer_id: "vm",
  image_status: "ready", spec_os: "Debian 13", spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "ready",
  guest_ready_ms: 4000, viewport_state: "agent_active", viewport_issue: null, display_available: false, display_attached: true,
  display_config: { width_px: 1440, height_px: 900 }, display_error: null, display_setup_error: null, control_owner: "agent",
  input_available: true, agent_busy: true, active_task: null,
};
const props = (patch: Partial<StatusPayload> = {}, rest: Partial<ComputerPanelProps> = {}): ComputerPanelProps => {
  const next = { ...status, ...patch };
  return {
    model: computerModel({ connected: true, native: true, status: next, events: [] }),
    level: "side", status: next, slotEnabled: false, snapshot: null, steps: [], busy: false, managing: false, error: null, moving: false, focusOnOpen: false,
    onCommand: vi.fn(), onManage: vi.fn(), onLevel: vi.fn(), onClose: vi.fn(), onSlotError: vi.fn(), onDismissError: vi.fn(),
    loadBootLog: vi.fn().mockResolvedValue({ available: false, total_lines: 0, tail: [] }), ...rest,
  };
};
const picture = (patch: Partial<Snapshot> = {}): Snapshot => ({ src: "data:image/png;base64,AAAA", at: Date.now(), error: null, loading: false, refresh: vi.fn(), ...patch });

describe("ComputerPanel", () => {
  it("keeps the same slot element from Side to Full", () => {
    const view = render(<ComputerPanel {...props({ display_available: true })} />);
    const slot = document.querySelector("[data-framebuffer-slot]");
    expect(slot).toBeTruthy();
    view.rerender(<ComputerPanel {...props({ display_available: true }, { level: "focus" })} />);
    view.rerender(<ComputerPanel {...props({ display_available: true }, { level: "full" })} />);
    expect(document.querySelector("[data-framebuffer-slot]")).toBe(slot);
  });

  it("says who has control and offers the handoff", () => {
    const onCommand = vi.fn();
    const view = render(<ComputerPanel {...props({}, { onCommand, slotEnabled: true })} />);
    expect(screen.getByText("Pegoles is using it")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));
    expect(onCommand).toHaveBeenCalledWith("take");
    view.rerender(<ComputerPanel {...props({ control_owner: "user", viewport_state: "user_controlled" }, { onCommand, level: "full" })} />);
    expect(screen.getByText("You have control", { selector: ".strip__owner span" })).toBeTruthy();
    expect(screen.getByText("⌃⌥⎋")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Give control to Pegoles" }));
    expect(onCommand).toHaveBeenLastCalledWith("return");
  });

  it("shows a real snapshot when there is no live view, and says how fresh it is", () => {
    const snapshot = picture();
    render(<ComputerPanel {...props({}, { snapshot, level: "focus" })} />);
    expect(screen.getByRole("img", { name: /Latest snapshot/ })).toBeTruthy();
    expect(screen.getByText(/Snapshot · just now/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Refresh snapshot" }));
    expect(snapshot.refresh).toHaveBeenCalledOnce();
  });

  it("is honest when there is neither a live view nor a picture", () => {
    render(<ComputerPanel {...props({}, { snapshot: picture({ src: null, error: "no frame" }) })} />);
    expect(screen.getByText("Its screen isn’t available")).toBeTruthy();
    expect(screen.getByText(/couldn’t capture its screen/)).toBeTruthy();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("uses the model's word in the header, never a word of its own", () => {
    render(<ComputerPanel {...props({ control_owner: "none", viewport_state: "ready", display_attached: false })} />);
    expect(screen.getByText("Ready", { selector: ".computer__state" })).toBeTruthy();
    expect(screen.queryByText("Running")).toBeNull();
  });

  it("changes level from its own header, and leaves Full back to the task", () => {
    const onLevel = vi.fn();
    const view = render(<ComputerPanel {...props({}, { onLevel })} />);
    fireEvent.click(screen.getByRole("button", { name: "Focus its computer" }));
    expect(onLevel).toHaveBeenLastCalledWith("focus");
    fireEvent.click(screen.getByRole("button", { name: "Fill the window" }));
    expect(onLevel).toHaveBeenLastCalledWith("full");
    view.rerender(<ComputerPanel {...props({}, { onLevel, level: "full" })} />);
    expect(screen.queryByRole("button", { name: "Close computer" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Task/ }));
    expect(onLevel).toHaveBeenLastCalledWith("focus");
  });

  it("puts the ways to intervene under the screen, and says why taking over isn't offered", () => {
    const onCommand = vi.fn();
    render(<ComputerPanel {...props({ display_attached: false }, { onCommand })} />);
    const controls = screen.getByRole("group", { name: "Computer controls" });
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(onCommand).toHaveBeenCalledWith("pause");
    expect(controls.textContent).toContain("Stop computer");
    expect(screen.getByText(/Taking over needs its live screen/)).toBeTruthy();
  });

  it("draws the off computer inside the same frame the screen will use", () => {
    render(<ComputerPanel {...props({ computer_state: "stopped", viewport_state: "off", control_owner: "none" })} />);
    expect(document.querySelector(".screen-frame .computer__screen .machine-state")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start computer" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Focus its computer" })).toBeNull();
  });

  it("lists what Pegoles last did on it", () => {
    render(<ComputerPanel {...props({}, { steps: [{ id: "a", label: "Clicking", outcome: "running", at: "2026-09-26T00:00:00Z", capability: "Computer" }] })} />);
    expect(screen.getByRole("heading", { name: "Latest on this computer" })).toBeTruthy();
    expect(screen.getByText("now")).toBeTruthy();
  });

  it("keeps the native view hidden while the workspace recomposes", async () => {
    vi.useFakeTimers();
    const geometry = vi.spyOn(api, "setDisplayGeometry").mockResolvedValue(undefined);
    vi.spyOn(api, "detachDisplay").mockResolvedValue(undefined);
    const view = render(<ComputerPanel {...props({ display_available: true }, { slotEnabled: true, moving: true })} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(geometry).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false, animate_ms: 0 }));
    view.rerender(<ComputerPanel {...props({ display_available: true }, { slotEnabled: true, moving: false })} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(geometry).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
    await act(async () => { view.unmount(); await vi.runAllTimersAsync(); });
    vi.useRealTimers();
  });

  it("asks inline before resetting or removing its computer", () => {
    const onManage = vi.fn();
    render(<ComputerPanel {...props({ computer_state: "stopped", viewport_state: "off", control_owner: "none" }, { onManage })} />);
    const reset = screen.getByRole("button", { name: "Reset computer…" });
    fireEvent.click(reset);
    const question = screen.getByRole("group", { name: "Reset its computer?" });
    expect(question.textContent).toMatch(/Everything done inside it is erased/);
    // The safe answer has focus; Escape takes the question back without doing anything.
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.keyDown(question, { key: "Escape" });
    expect(screen.queryByRole("group", { name: "Reset its computer?" })).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Reset computer…" }));
    expect(onManage).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Remove computer…" }));
    expect(screen.getByRole("group", { name: "Remove its computer?" }).textContent).toMatch(/disk and everything on it are deleted/);
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onManage).toHaveBeenCalledExactlyOnceWith("remove");
  });

  it("won't reset or remove its computer while a task is running on it", () => {
    render(<ComputerPanel {...props({ active_task: "t1" })} />);
    expect((screen.getByRole("button", { name: "Reset computer…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Remove computer…" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Stop the running task to reset or remove its computer.")).toBeTruthy();
  });

  it("closes an open question when a task starts under it", () => {
    const view = render(<ComputerPanel {...props()} />);
    fireEvent.click(screen.getByRole("button", { name: "Reset computer…" }));
    expect(screen.getByRole("group", { name: "Reset its computer?" })).toBeTruthy();
    view.rerender(<ComputerPanel {...props({ active_task: "t1" })} />);
    expect(screen.queryByRole("group", { name: "Reset its computer?" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reset" })).toBeNull();
  });

  it("has nothing to reset before its computer exists", () => {
    render(<ComputerPanel {...props({ computer_created: false, computer_state: null, viewport_state: "off", control_owner: "none" })} />);
    expect(screen.queryByRole("button", { name: /Reset computer|Remove computer/ })).toBeNull();
  });

  it("keeps specs behind Details", () => {
    render(<ComputerPanel {...props({ computer_state: "stopped", viewport_state: "off", control_owner: "none" })} />);
    expect(screen.getByText("Off", { selector: ".machine-state__headline" })).toBeTruthy();
    const summary = screen.getByText("Details", { selector: "summary" });
    expect(summary.closest("details")?.open).toBe(false);
    expect(summary.closest("details")?.textContent).toContain("Debian 13 · arm64 · 2 CPU");
  });
});

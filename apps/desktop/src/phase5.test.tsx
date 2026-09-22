import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ActivityTimeline } from "./components/ActivityTimeline";
import { overlayPoint } from "./lib/agentCursorFeed";
import { currentActionFor } from "./lib/currentAction";
import type { AgentEvent, StatusPayload } from "./lib/tauri";

afterEach(cleanup);

const base: StatusPayload = {
  viewport_state: "agent_active",
  viewport_issue: null,
  display_available: true,
  display_attached: true,
  display_config: { width_px: 1440, height_px: 900 },
  display_error: null,
  display_setup_error: null,
  control_owner: "agent",
  core: "running",
  model: "not_configured",
  backend: "real",
  computer_created: true,
  computer_state: "running",
  computer_id: "vm-1",
  image_status: "ready",
  spec_os: "Debian",
  spec_arch: "arm64",
  spec_vcpus: 4,
  spec_ram_mb: 4096,
  guest_state: "ready",
  guest_ready_ms: 4000,
  input_available: true,
  agent_busy: true,
};

function started(id: string, type: string, extra: object = {}): AgentEvent {
  return {
    type: "action_started",
    action_id: id,
    request: {
      action_id: id,
      task_id: "t",
      computer_id: "vm-1",
      action: { type, ...extra },
      requested_at: new Date().toISOString(),
    },
    at: new Date().toISOString(),
  } as AgentEvent;
}

describe("overlayPoint (real slot geometry)", () => {
  it("maps normalized positions through the live slot rect", () => {
    document.body.innerHTML =
      '<div class="computer-viewport-wrap"><div data-framebuffer-slot=""></div></div>';
    const wrap = document.querySelector(".computer-viewport-wrap") as HTMLElement;
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

describe("currentActionFor with live actions", () => {
  it("names typing from a live type_text action", () => {
    const events = [started("a1", "type_text", { text: "hi", sensitive: false })];
    const action = currentActionFor(base, true, "running", events[0], events);
    expect(action?.label).toBe("Pegoles is typing");
    expect(action?.working).toBe(true);
  });

  it("names scrolling and falls back when the action settles", () => {
    const events = [started("a2", "scroll", { x: 0.5, y: 0.5, delta_x: 0, delta_y: 3 })];
    expect(currentActionFor(base, true, "running", events[0], events)?.label).toBe(
      "Pegoles is scrolling",
    );
    const settled: AgentEvent[] = [
      ...events,
      {
        type: "action_completed",
        request: (events[0] as { request: unknown }).request,
        result: { action_id: "a2", outcome: "executed", success: true, message: "ok", duration_ms: 3 },
      } as AgentEvent,
    ];
    // No live action left: viewport fallback (agent_active + latest event).
    expect(currentActionFor(base, true, "running", settled[1], settled)?.label).toContain(
      "Pegoles is working",
    );
  });
});

describe("ActivityTimeline summarizes lifecycles", () => {
  it("collapses one action to one row and hides bare moves", () => {
    const events: AgentEvent[] = [
      started("m1", "move_pointer", { x: 0.1, y: 0.1 }),
      started("c1", "click", { x: 0.5, y: 0.5, button: "primary" }),
      {
        type: "action_completed",
        request: { action_id: "c1", action: { type: "click" } },
        result: { action_id: "c1", outcome: "executed", success: true, message: "ok", duration_ms: 5 },
      } as unknown as AgentEvent,
    ];
    render(<ActivityTimeline events={events} />);
    expect(screen.getByText("Clicking")).toBeTruthy();
    expect(screen.queryByText(/Moving pointer/)).toBeNull();
  });
});

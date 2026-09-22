import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { currentActionFor } from "./lib/currentAction";
import type { StatusPayload } from "./lib/tauri";

afterEach(cleanup);

const base: StatusPayload = {
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
  viewport_state: "agent_active",
  viewport_issue: null,
  display_available: true,
  display_attached: true,
  display_config: { width_px: 1440, height_px: 900 },
  display_error: null,
  display_setup_error: null,
  control_owner: "agent",
  input_available: true,
  agent_busy: true,
};

describe("currentActionFor (real state only)", () => {
  it("returns null when disconnected", () => {
    expect(currentActionFor(base, false, "running", null)).toBeNull();
  });

  it("names user control explicitly", () => {
    const action = currentActionFor({ ...base, control_owner: "user", viewport_state: "user_controlled" }, true, "running", null);
    expect(action?.tone).toBe("user");
    expect(action?.label).toMatch(/controlling/);
    expect(action?.working).toBe(false);
  });

  it("names approval without inventing payload", () => {
    const action = currentActionFor({ ...base, viewport_state: "ready" }, true, "waiting_for_approval", null);
    expect(action?.tone).toBe("waiting");
    expect(action?.label).toMatch(/approval/);
  });

  it("stays honest when idle with no task", () => {
    expect(currentActionFor({ ...base, viewport_state: "ready" }, true, null, null)).toBeNull();
    expect(currentActionFor({ ...base, viewport_state: "off" }, true, "pending", null)).toBeNull();
  });

  it("reports agent activity from the viewport state", () => {
    const action = currentActionFor(base, true, "running", null);
    expect(action?.tone).toBe("active");
    expect(action?.working).toBe(true);
  });
});

describe("task workspace honesty", () => {
  it("renders the pending notice without fake progress", async () => {
    const { TaskWorkspace } = await import("./components/TaskWorkspace");
    render(
      <TaskWorkspace
        task={{ id: "t", title: "Test task", status: "pending", created_at: new Date().toISOString(), updated_at: new Date().toISOString() }}
        layout="split"
        onLayoutChange={() => undefined}
        onBack={() => undefined}
        status={base}
        connected
        events={[]}
        taskEvents={[]}
        computer={<div>computer</div>}
      />,
    );
    expect(screen.getByText(/Autonomous execution is not available yet/)).toBeTruthy();
    expect(screen.queryByText(/2m 14s/)).toBeNull();
  });
});

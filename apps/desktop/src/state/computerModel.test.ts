import { describe, expect, it } from "vitest";
import { computerModel, withCommandError, type ComputerInputs } from "./computerModel";
import type { AgentEvent, StatusPayload } from "../lib/tauri";

const status: StatusPayload = {
  core: "running", model: "not_configured", provider: "local", backend: "real", computer_created: true, computer_state: "running", computer_id: "vm",
  image_status: "ready", spec_os: "Debian 13", spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "ready",
  guest_ready_ms: 4000, viewport_state: "ready", viewport_issue: null, display_available: false, display_attached: false,
  display_config: { width_px: 1440, height_px: 900 }, display_error: null, display_setup_error: null, control_owner: "none",
  input_available: true, agent_busy: false, active_task: null,
};
const input = (patch: Partial<StatusPayload> = {}, rest: Partial<ComputerInputs> = {}): ComputerInputs => ({
  connected: true, native: true, status: { ...status, ...patch }, events: [], ...rest,
});

describe("computerModel", () => {
  it("is a status, not a monitor, when off", () => {
    const off = computerModel(input({ computer_created: false, computer_state: null, viewport_state: "off" }));
    expect(off).toMatchObject({ phase: "off", chip: "Off", running: false, primary: { command: "start", label: "Start computer" } });
    expect(off.body).toMatch(/when it needs it/);
  });

  it("keeps technical facts out of the headline and in specs", () => {
    const model = computerModel(input());
    expect(model.headline).not.toMatch(/Debian|arm64|CPU/);
    expect(model.specs).toEqual(["Debian 13", "arm64", "2 CPU", "1.5 GB memory", "1440 × 900"]);
  });

  it("shows only facts Core reports, with startup time once it is up", () => {
    expect(computerModel(input()).facts).toEqual([
      { label: "System", value: "Debian 13 · arm64" },
      { label: "Isolation", value: "Separate virtual machine" },
      { label: "Ready in", value: "4 s" },
    ]);
    const off = computerModel(input({ computer_created: false, computer_state: null, guest_ready_ms: 4000 }));
    expect(off.facts.map((fact) => fact.label)).toEqual(["System", "Isolation"]);
    expect(computerModel(input({ backend: "mock" })).facts).toContainEqual({ label: "Backend", value: "Simulated" });
    expect(computerModel({ ...input(), connected: false }).facts).toEqual([]);
  });

  it("says plainly when the sealed image isn't installed, and offers nothing that can't fix it", () => {
    const missing = computerModel(input({ computer_created: false, computer_state: null, image_status: "missing" }));
    expect(missing).toMatchObject({ phase: "needs-setup", chip: "Not installed", headline: "The Pegoles computer image isn’t installed on this Mac." });
    expect(missing.primary).toBeUndefined();
    expect(missing.devNote).toBe("Build it with scripts/build-guest-image (see docs/PROJECT_STATE.md).");
    const invalid = computerModel(input({ computer_created: false, computer_state: null, image_status: "invalid" }));
    expect(invalid).toMatchObject({ phase: "needs-setup", chip: "Image incomplete" });
    expect(invalid.primary).toBeUndefined();
    expect(invalid.devNote).toMatch(/^Rebuild it with scripts\/build-guest-image/);
    // A computer that already exists keeps working; the simulated backend needs no image.
    expect(computerModel(input({ image_status: "missing" })).phase).toBe("ready");
    expect(computerModel(input({ computer_created: false, computer_state: null, image_status: "missing", backend: "mock" })).phase).toBe("off");
  });

  it("walks through boot with real stages only", () => {
    const ready: AgentEvent = { type: "guest_runtime_ready", computer_id: "vm", ready_in_ms: 4200, at: "2026-09-24T10:00:00Z" };
    const booting = computerModel(input({ computer_state: "starting", viewport_state: "display_starting", guest_state: "connecting" }, { events: [ready] }));
    expect(booting.phase).toBe("starting");
    expect(booting.steps.map((step) => step.state)).toEqual(["done", "done", "active"]);
  });

  it("names ownership explicitly and offers takeover only with a real display", () => {
    expect(computerModel(input({ control_owner: "agent", viewport_state: "agent_active" }))).toMatchObject({ phase: "agent", owner: "agent", headline: "Pegoles has control", primary: undefined });
    expect(computerModel(input({ control_owner: "agent", display_attached: true })).primary).toEqual({ command: "take", label: "Take control" });
    expect(computerModel(input({ control_owner: "user", viewport_state: "user_controlled" }))).toMatchObject({ phase: "user", chip: "You have control", primary: { command: "return", label: "Give control to Pegoles" } });
  });

  it("talks to people when the machine fails", () => {
    const failed = computerModel(input({ computer_state: "error", viewport_state: "error" }));
    expect(failed).toMatchObject({ phase: "error", chip: "Can’t start", headline: "Computer couldn’t start.", primary: { label: "Try again" } });
    const off = computerModel(input({ computer_created: false, computer_state: null, viewport_state: "off" }));
    expect(withCommandError(off, true)).toMatchObject({ phase: "error", chip: "Can’t start", primary: { command: "start" } });
    expect(withCommandError(computerModel(input()), true).phase).toBe("ready");
  });

  it("is unavailable outside the desktop app", () => {
    expect(computerModel({ ...input(), connected: false, native: false })).toMatchObject({ phase: "unavailable", body: expect.stringMatching(/Open the Pegoles app/) });
  });
});

import { describe, expect, it, vi } from "vitest";
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

  it("offers to set up the image in the app when this build can download it", () => {
    const setup = { available: true, installing: false, stage: null, done: 0, total: 0, error: null, download_bytes: 561_846_260, disk_bytes: 3_221_225_472 };
    const offer = computerModel(input({ computer_created: false, computer_state: null, image_status: "missing", image_setup: setup }));
    expect(offer).toMatchObject({ phase: "needs-setup", chip: "Not installed", primary: { command: "install", label: "Set up computer" } });
    expect(offer.body).toBe("It downloads once (562 MB), is checked before use and needs about 3.2 GB of disk space.");
    expect(offer.devNote).toBeUndefined();
    const repair = computerModel(input({ computer_created: false, computer_state: null, image_status: "invalid", image_setup: { ...setup, error: "Setup was cancelled. It resumes where it stopped." } }));
    expect(repair).toMatchObject({ chip: "Image incomplete", primary: { command: "install", label: "Repair computer" } });
    expect(repair.body).toMatch(/^Setup was cancelled\. It resumes where it stopped\. It downloads once/);
  });

  it("shows only real setup progress, and the way to stop it", () => {
    const base = { available: true, installing: true, error: null, download_bytes: 561_846_260, disk_bytes: 3_221_225_472 };
    const at = (stage: "downloading" | "verifying" | "unpacking" | "finalizing", done: number, total: number) =>
      computerModel(input({ computer_created: false, computer_state: null, image_status: "missing", image_setup: { ...base, stage, done, total } }));
    const downloading = at("downloading", 280_923_130, 561_846_260);
    expect(downloading).toMatchObject({ phase: "needs-setup", chip: "Setting up", transitioning: true, primary: { command: "cancel-install", label: "Cancel" } });
    expect(downloading.body).toBe("Downloading… 50% of 562 MB");
    expect(at("verifying", 1, 4).body).toBe("Checking the download… 25%");
    expect(at("unpacking", 3, 3).body).toBe("Unpacking… 100%");
    expect(at("finalizing", 0, 1).body).toBe("Finishing…");
  });

  it("keeps developer build steps out of production: the product never points people at repository scripts", () => {
    vi.stubEnv("DEV", false);
    try {
      for (const image_status of ["missing", "invalid"] as const) {
        const model = computerModel(input({ computer_created: false, computer_state: null, image_status }));
        expect(model.phase).toBe("needs-setup");
        expect(model.devNote).toBeUndefined();
        expect(JSON.stringify(model)).not.toMatch(/scripts\/|\.sh\b|docs\/|cargo /);
      }
    } finally {
      vi.unstubAllEnvs();
    }
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

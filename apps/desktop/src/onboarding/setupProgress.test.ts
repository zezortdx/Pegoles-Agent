import { describe, expect, it } from "vitest";
import type { StatusPayload } from "../lib/tauri";
import type { LocalView } from "../state/localModel";
import { planFor, setupProgress } from "./setupProgress";

const model = {
  id: "mai-ui-2b-6bit", display_name: "MAI-UI 2B", family: "MaiUi", parameters: "2B", quantization: "6bit", size_bytes: 2_000_000_000,
  license: "apache-2.0", source: "", state: "not_installed" as const, partial_bytes: null, invalid_reason: null, downloadable: true, recommended_min_ram_gb: null,
};
const base = { model, problem: null, running: false, footprintBytes: null };
const notSetUp: LocalView = { ...base, stage: "not-set-up" };
const downloading = (done: number): LocalView => ({ ...base, stage: "preparing", step: "downloading", doneBytes: done, totalBytes: model.size_bytes });
const ready: LocalView = { ...base, stage: "ready" };

const status = (patch: Partial<StatusPayload> = {}): StatusPayload => ({
  core: "running", model: "not_configured", provider: "local", backend: "real", computer_created: false, computer_state: null, computer_id: null,
  image_status: "missing", spec_os: "", spec_arch: "", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "unavailable", guest_ready_ms: null,
  viewport_state: "off", viewport_issue: null, display_available: false, display_attached: false, display_config: null, display_error: null,
  display_setup_error: null, control_owner: "none", input_available: false, agent_busy: false, active_task: null,
  image_setup: { available: true, installing: false, stage: null, done: 0, total: 0, error: null, download_bytes: 500_000_000, disk_bytes: 3e9 },
  ...patch,
});

const plan = planFor(notSetUp, status());
const run = { running: true, paused: false, commandError: null };

describe("Set up Pegoles as one job", () => {
  it("before starting: nothing measured, every part listed", () => {
    const p = setupProgress({ plan, local: notSetUp, status: status(), running: false, paused: false, commandError: null });
    expect(p.phase).toBe("idle");
    expect(p.totalBytes).toBe(2_500_000_000);
    expect(p.percent).toBe(0);
    expect(p.steps.map((s) => s.id)).toEqual(["model", "computer", "verify", "finish"]);
  });

  it("counts the model's real bytes against the whole download", () => {
    const p = setupProgress({ plan, local: downloading(1_000_000_000), status: status(), ...run });
    expect(p.phase).toBe("working");
    expect(p.headline).toBe("Downloading AI model");
    expect(p.percent).toBe(40);
    expect(p.transferring).toBe(true);
    expect(p.steps[0].state).toBe("active");
  });

  it("the total never jumps when a part finishes", () => {
    const image = status({ image_setup: { available: true, installing: true, stage: "downloading", done: 250_000_000, total: 500_000_000, error: null, download_bytes: 500_000_000, disk_bytes: 3e9 } });
    const p = setupProgress({ plan, local: ready, status: image, ...run });
    expect(p.totalBytes).toBe(2_500_000_000);
    expect(p.doneBytes).toBe(2_250_000_000);
    expect(p.headline).toBe("Preparing your computer");
    expect(p.steps.find((s) => s.id === "model")?.state).toBe("done");
  });

  it("then the first start, then done", () => {
    const booting = status({ image_status: "ready", computer_created: true, computer_state: "starting", guest_state: "connecting" });
    expect(setupProgress({ plan, local: ready, status: booting, ...run }).headline).toBe("Almost ready");
    const up = status({ image_status: "ready", computer_created: true, computer_state: "running", guest_state: "ready" });
    const done = setupProgress({ plan, local: ready, status: up, ...run });
    expect(done.phase).toBe("done");
    expect(done.percent).toBe(100);
    expect(done.steps.every((s) => s.state === "done")).toBe(true);
  });

  it("a failure keeps its kind and the technical text", () => {
    const failed: LocalView = { ...base, stage: "failed", error: "not enough disk space: 2 bytes needed, 1 free", errorKind: "disk_space", doneBytes: 5, totalBytes: model.size_bytes };
    const p = setupProgress({ plan, local: failed, status: status(), ...run });
    expect(p.phase).toBe("failed");
    expect(p.error).toEqual({ kind: "disk_space", technical: "not enough disk space: 2 bytes needed, 1 free" });
    const net = setupProgress({ plan, local: ready, status: status({ image_setup: { available: true, installing: false, stage: null, done: 0, total: 0, error: "network error: connection reset", download_bytes: 500_000_000, disk_bytes: 3e9 } }), ...run });
    expect(net.error?.kind).toBe("network");
  });

  it("a computer that starts but never answers is a failure, not an endless wait", () => {
    const stuck = status({ image_status: "ready", computer_created: true, computer_state: "running", guest_state: "error" });
    const p = setupProgress({ plan, local: ready, status: stuck, ...run });
    expect(p.phase).toBe("failed");
    expect(p.error?.technical).toContain("guest runtime error");
  });

  it("a pause is the person's, and resumable", () => {
    const paused: LocalView = { ...base, stage: "paused", doneBytes: 700_000_000, totalBytes: model.size_bytes };
    const p = setupProgress({ plan, local: paused, status: status(), running: false, paused: true, commandError: null });
    expect(p.phase).toBe("paused");
    expect(p.resumable).toBe(true);
  });

  it("already set up: nothing to download", () => {
    const up = status({ image_status: "ready" });
    const onlyBoot = planFor(ready, up);
    expect(onlyBoot).toEqual({ model: false, image: false });
    expect(setupProgress({ plan: onlyBoot, local: ready, status: up, running: false, paused: false, commandError: null }).steps.map((s) => s.id)).toEqual(["verify", "finish"]);
  });
});

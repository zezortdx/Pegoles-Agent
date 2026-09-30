import { describe, expect, it } from "vitest";
import type { SystemCheck } from "../lib/tauri";
import { checkVerdict, technicalReport } from "./systemCheck";

const GIB = 2 ** 30;
const mac: SystemCheck = {
  platform: "macos", os_name: "macOS 15.5", os_supported: true, os_minimum: "macOS 14", architecture: "arm64", architecture_supported: true,
  virtualization: { state: "ready", fixable: false, technical: "Apple Virtualization framework" },
  memory_bytes: 16 * GIB, memory_minimum_bytes: 8 * GIB, memory_recommended_bytes: 16 * GIB,
  disk_free_bytes: 40e9, disk_needed_bytes: 6e9,
  acceleration: { kind: "metal", device: "Apple M4 Pro", technical: "Metal" },
  runtime_ready: true, runtime_problem: null, model_ready: false, image_ready: false,
};
const windows: SystemCheck = {
  ...mac, platform: "windows", os_name: "Windows 11 Home", os_minimum: "Windows 11", architecture: "x86_64",
  virtualization: { state: "needs_enable", fixable: true, technical: "VirtualMachinePlatform: Disabled" },
  acceleration: { kind: "vulkan", device: "NVIDIA GeForce RTX 4060", technical: "Vulkan" },
};

describe("system check wording", () => {
  it("a ready Mac passes with plain rows", () => {
    const v = checkVerdict(mac);
    expect(v.canContinue).toBe(true);
    expect(v.fix).toBeNull();
    expect(v.rows.map((row) => row.title)).toEqual([
      "macOS 15.5", "Virtualization", "16 GB memory", "Hardware acceleration", "Enough disk space", "AI runtime included",
    ]);
    expect(v.rows.every((row) => row.tone === "ok")).toBe(true);
  });

  it("offers to turn virtualization on, explaining why, without jargon", () => {
    const v = checkVerdict(windows);
    expect(v.canContinue).toBe(false);
    expect(v.fix).toEqual({ kind: "enable-virtualization" });
    const row = v.rows.find((r) => r.id === "virtualization");
    expect(row?.tone).toBe("action");
    expect(row?.detail).toMatch(/its own isolated computer/);
    expect(row?.detail).not.toMatch(/HCS|Hyper-V|0x8037/);
  });

  it("firmware-disabled virtualization is a person's step, and a restart is offered when pending", () => {
    expect(checkVerdict({ ...windows, virtualization: { state: "firmware_disabled", fixable: false, technical: "" } }).fix).toEqual({ kind: "firmware" });
    expect(checkVerdict({ ...windows, virtualization: { state: "restart_pending", fixable: false, technical: "" } }).fix).toEqual({ kind: "restart" });
  });

  it("low memory warns but never blocks; missing disk space blocks with the exact amount", () => {
    const low = checkVerdict({ ...mac, memory_bytes: 8 * GIB, disk_free_bytes: 4e9 });
    expect(low.rows.find((r) => r.id === "memory")?.tone).toBe("warn");
    const disk = low.rows.find((r) => r.id === "disk");
    expect(disk?.tone).toBe("blocked");
    expect(disk?.detail).toContain("Free up 2.0 GB");
    expect(low.canContinue).toBe(false);
  });

  it("CPU-only inference is a note, not a blocker", () => {
    const v = checkVerdict({ ...windows, virtualization: { state: "ready", fixable: false, technical: "" }, acceleration: { kind: "cpu", device: null, technical: "" } });
    expect(v.canContinue).toBe(true);
    expect(v.rows.find((r) => r.id === "acceleration")?.tone).toBe("warn");
  });

  it("an Intel Mac is told plainly, once", () => {
    const v = checkVerdict({ ...mac, architecture: "x86_64", architecture_supported: false, acceleration: { kind: "none", device: null, technical: "" }, runtime_ready: false });
    expect(v.canContinue).toBe(false);
    expect(v.rows.some((r) => r.id === "runtime")).toBe(false);
    expect(v.rows[0].detail).toMatch(/Apple silicon/);
  });

  it("technical details carry the facts", () => {
    const report = technicalReport(windows, checkVerdict(windows));
    expect(report).toContain("VirtualMachinePlatform: Disabled");
    expect(report).toContain("Platform: windows (x86_64)");
  });
});

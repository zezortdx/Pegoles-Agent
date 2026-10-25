import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { api, type OnboardingState, type StatusPayload, type SystemCheck } from "../lib/tauri";
import { intelligenceOf } from "../state/intelligenceFixture";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

const GIB = 2 ** 30;
const status: StatusPayload = {
  core: "running", model: "configured", provider: "local", backend: "real", computer_created: true, computer_state: "running", computer_id: "vm-1",
  image_status: "ready", spec_os: "Debian 13", spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "ready", guest_ready_ms: 3200,
  viewport_state: "ready", viewport_issue: null, display_available: false, display_attached: false, display_config: null, display_error: null,
  display_setup_error: null, control_owner: "none", input_available: true, agent_busy: false, active_task: null,
};
const windowsCheck: SystemCheck = {
  platform: "windows", os_name: "Windows 11 Home (24H2)", os_supported: true, os_minimum: "Windows 11", architecture: "x86_64", architecture_supported: true,
  virtualization: { state: "needs_enable", fixable: true, technical: "VirtualMachinePlatform: Disabled" },
  memory_bytes: 16 * GIB, memory_minimum_bytes: 8 * GIB, memory_recommended_bytes: 16 * GIB, disk_free_bytes: 90e9, disk_needed_bytes: 6e9,
  acceleration: { kind: "cpu", device: null, technical: "AVX2" }, runtime_ready: true, runtime_problem: null, model_ready: false, image_ready: false,
};

function boot(step: OnboardingState["step"]) {
  Object.defineProperty(window, "__PEGOLES_BOOT__", {
    value: { onboarding: { version: 1, step, completed: false, restart_requested: false } }, configurable: true,
  });
}

beforeEach(() => {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("min-width") || query.includes("reduce"), media: query, addEventListener: () => undefined, removeEventListener: () => undefined,
  }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(api, "getStatus").mockResolvedValue(status);
  vi.spyOn(api, "listEvents").mockResolvedValue([]);
  vi.spyOn(api, "listTasks").mockResolvedValue([]);
  vi.spyOn(api, "getModelSettings").mockResolvedValue({ configured: false, key_source: null, model: "claude-opus-5", effort: "high", models: [], efforts: [] });
  vi.spyOn(api, "getIntelligence").mockResolvedValue(intelligenceOf());
  vi.spyOn(api, "getHostCapabilities").mockResolvedValue({ platform: "windows", architecture: "x86_64", backend: "windows-hcs", backend_available: true, backend_detail: "", guest_transport: "hyperv_socket", guest_transport_available: true, required_setup: [], supported: true });
  vi.spyOn(api, "suggestedEffects").mockResolvedValue({ tier: "reduced" });
  vi.spyOn(api, "accessibilityDisplay").mockResolvedValue({ reduce_transparency: false, increase_contrast: false });
  vi.spyOn(api, "captureScreen").mockRejectedValue(new Error("no screen in tests"));
  vi.spyOn(api, "setOnboardingStep").mockImplementation(async (step) => ({ version: 1, step, completed: false, restart_requested: false }));
  vi.spyOn(api, "systemCheck").mockResolvedValue(windowsCheck);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as unknown as Record<string, unknown>).__PEGOLES_BOOT__; });

describe("first-run onboarding", () => {
  it("opens instead of the main window, and walks welcome → how it works → the check, saving each step", async () => {
    boot("welcome");
    vi.spyOn(api, "getOnboarding").mockResolvedValue({ version: 1, step: "welcome", completed: false, restart_requested: false });
    render(<App />);
    expect(screen.getByRole("heading", { level: 1, name: "Welcome to Pegoles" })).toBeTruthy();
    expect(screen.queryByRole("complementary", { name: "Sidebar" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Get started" }));
    expect(await screen.findByRole("heading", { level: 1, name: "How Pegoles works" })).toBeTruthy();
    expect(api.setOnboardingStep).toHaveBeenCalledWith("how");
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Checking this PC" })).toBeTruthy();
    const checks = await screen.findByRole("list", { name: "System check" });
    await waitFor(() => expect(within(checks).getByText("Virtualization needs to be turned on")).toBeTruthy());
    // Nothing technical in the rows themselves.
    expect(within(checks).queryByText(/VirtualMachinePlatform|HCS|0x8037/)).toBeNull();
  });

  it("explains before Windows asks for administrator permission, and only asks after Continue", async () => {
    boot("check");
    vi.spyOn(api, "getOnboarding").mockResolvedValue({ version: 1, step: "check", completed: false, restart_requested: false });
    const fix = vi.spyOn(api, "fixVirtualization").mockResolvedValue("restart_required");
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Fix automatically" }));
    expect(screen.getByText(/Windows will now ask for administrator permission/)).toBeTruthy();
    expect(fix).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(fix).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("heading", { level: 2, name: "One restart needed" })).toBeTruthy();
    const restart = vi.spyOn(api, "restartToFinishSetup").mockResolvedValue(null);
    fireEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(restart).not.toHaveBeenCalled();
  });

  it("finishes with a first task that really starts", async () => {
    boot("ready");
    vi.spyOn(api, "getOnboarding").mockResolvedValue({ version: 1, step: "ready", completed: false, restart_requested: false });
    const finish = vi.spyOn(api, "finishOnboarding").mockResolvedValue({ version: 1, step: "ready", completed: true, restart_requested: false });
    const created = { id: "t1", title: "x", status: "pending" as const, created_at: "2026-09-27T10:00:00Z", updated_at: "2026-09-27T10:00:00Z" };
    const create = vi.spyOn(api, "createTask").mockResolvedValue(created);
    const start = vi.spyOn(api, "runTask").mockResolvedValue(null);
    render(<App />);
    // Core has answered (as it has long before anyone reaches this screen).
    await waitFor(() => expect(api.listTasks).toHaveBeenCalled());
    await act(async () => { await Promise.resolve(); });
    fireEvent.click(screen.getByRole("radio", { name: /Work something out/ }));
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Try your first task" })); });
    expect(finish).toHaveBeenCalled();
    await waitFor(() => expect(create).toHaveBeenCalledWith(expect.stringContaining("1234 × 5678")));
    await waitFor(() => expect(start).toHaveBeenCalledWith("t1"));
  });
});

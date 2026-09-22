import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { api, type AgentTask, type StatusPayload } from "./lib/tauri";

const bridge = vi.hoisted(() => ({ native: true }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => bridge.native, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
const status: StatusPayload = {
  core: "running", model: "not_configured", backend: "real", computer_created: false,
  computer_state: null, computer_id: null, image_status: "ready", spec_os: "Debian",
  spec_arch: "arm64", spec_vcpus: 4, spec_ram_mb: 4096, guest_state: "unavailable",
  guest_ready_ms: null, viewport_state: "off", viewport_issue: null, display_available: false,
  display_attached: false, display_config: null, display_error: null, display_setup_error: null, control_owner: "none",
  input_available: false, agent_busy: false,
};
const task: AgentTask = { id: "task-1", title: "Organize my notes", status: "pending", created_at: "2026-09-22T10:00:00Z", updated_at: "2026-09-22T10:00:00Z" };

beforeEach(() => {
  bridge.native = true;
  localStorage.clear();
  vi.spyOn(api, "getStatus").mockResolvedValue(status);
  vi.spyOn(api, "listEvents").mockResolvedValue([]);
  vi.spyOn(api, "listTasks").mockResolvedValue([]);
  vi.spyOn(api, "getImageStatus").mockResolvedValue({ status: "ready", preparing: false, stage: null, downloaded: 0, total: 0, error: null });
  vi.spyOn(api, "getHostCapabilities").mockResolvedValue({ platform: "macos", architecture: "arm64", backend: "real", backend_available: true, backend_detail: "", guest_transport: "virtio_socket", guest_transport_available: true, required_setup: [], supported: true });
  vi.spyOn(api, "suggestedEffects").mockResolvedValue({ tier: "reduced" });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("desktop shell", () => {
  it("browser preview stays disconnected and never invokes the backend", () => {
    bridge.native = false;
    render(<App />);
    expect((screen.getByRole("textbox", { name: "New task" }) as HTMLInputElement).disabled).toBe(true);
    expect(api.getStatus).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Computer" }));
    expect(screen.getByRole("heading", { name: "Computer unavailable" })).toBeTruthy();
  });

  it("submits to Core and keeps pending tasks honest through workspace layouts", async () => {
    vi.spyOn(api, "createTask").mockImplementation(async () => {
      vi.mocked(api.listTasks).mockResolvedValue([task]);
      return task;
    });
    render(<App />);
    const input = screen.getByRole("textbox", { name: "New task" });
    await waitFor(() => expect((input as HTMLInputElement).disabled).toBe(false));
    fireEvent.change(input, { target: { value: task.title } });
    fireEvent.submit(screen.getByRole("form", { name: "New task" }));
    expect(await screen.findByRole("heading", { name: task.title })).toBeTruthy();
    expect(api.createTask).toHaveBeenCalledExactlyOnceWith(task.title);
    expect(screen.getByText(/Task saved. Autonomous execution is not available yet/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Focus" }));
    expect(screen.getByRole("heading", { name: "Pegoles Computer" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Compact" }));
    expect(screen.getByRole("heading", { name: "Pegoles Computer" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Split" }));
    expect(screen.getByRole("heading", { name: "Pegoles Computer" })).toBeTruthy();
    expect(document.querySelector('[data-presence="thinking"]')).toBeNull();
  });

  it("uses the reported viewport state and routes human control through Core", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, computer_created: true, computer_state: "running", viewport_state: "user_controlled", control_owner: "user" });
    vi.spyOn(api, "returnControl").mockResolvedValue(undefined);
    render(<App />);
    await screen.findByText("Core connected");
    fireEvent.click(screen.getByRole("button", { name: "Computer" }));
    expect(document.querySelector('.pg-viewport')?.getAttribute("data-state")).toBe("user_controlled");
    expect(screen.getByText("You're controlling Pegoles Computer")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Return to Pegoles" }));
    await waitFor(() => expect(api.returnControl).toHaveBeenCalledOnce());
  });

  it("retains a failed submission draft and reports the error", async () => {
    vi.spyOn(api, "createTask").mockRejectedValue(new Error("Task storage unavailable"));
    render(<App />);
    const input = screen.getByRole("textbox", { name: "New task" }) as HTMLInputElement;
    await waitFor(() => expect(input.disabled).toBe(false));
    fireEvent.change(input, { target: { value: task.title } });
    fireEvent.submit(screen.getByRole("form", { name: "New task" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(input.value).toBe(task.title);
    expect(screen.queryByRole("heading", { name: task.title })).toBeNull();
  });

  it("persists effects without changing the system motion preference", async () => {
    render(<App />);
    await screen.findByText("Core connected");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "minimal" } });
    expect(document.documentElement.dataset.effectsTier).toBe("minimal");
    expect(localStorage.getItem("pegoles.effects")).toBe("minimal");
  });
});

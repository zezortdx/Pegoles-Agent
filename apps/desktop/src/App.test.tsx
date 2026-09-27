import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";
import App from "./App";
import { api, MODEL_INSTALL_EVENT, type AgentEvent, type AgentTask, type ModelSettings, type StatusPayload } from "./lib/tauri";
import { installOf, intelligenceOf, MAI } from "./state/intelligenceFixture";

const bridge = vi.hoisted(() => ({ native: true }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => bridge.native, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

/** Deliver a setup event to whoever listens for it (useIntelligence). */
const emitInstall = (payload: unknown) => act(() => {
  for (const [name, handler] of vi.mocked(listen).mock.calls) {
    if (name === MODEL_INSTALL_EVENT) (handler as (event: { payload: unknown }) => void)({ payload });
  }
});

const status: StatusPayload = {
  core: "running", model: "not_configured", provider: "local", backend: "real", computer_created: false,
  computer_state: null, computer_id: null, image_status: "ready", spec_os: "Debian 13",
  spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "unavailable",
  guest_ready_ms: null, viewport_state: "off", viewport_issue: null, display_available: false,
  display_attached: false, display_config: null, display_error: null, display_setup_error: null, control_owner: "none",
  input_available: false, agent_busy: false, active_task: null,
};
const running: Partial<StatusPayload> = { computer_created: true, computer_state: "running", viewport_state: "ready", guest_state: "ready" };
const task: AgentTask = { id: "task-1", title: "Organize my notes", status: "pending", created_at: "2026-09-22T10:00:00Z", updated_at: "2026-09-22T10:00:00Z" };
const noModel: ModelSettings = {
  configured: false, key_source: null, model: "claude-opus-5", effort: "high",
  models: ["claude-opus-5", "claude-sonnet-5", "claude-opus-5-5"], efforts: ["low", "medium", "high", "xhigh", "max"],
};
const withKey: ModelSettings = { ...noModel, configured: true, key_source: "keychain" };

beforeEach(() => {
  bridge.native = true;
  localStorage.clear();
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("min-width"), media: query, addEventListener: () => undefined, removeEventListener: () => undefined,
  }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(api, "getStatus").mockResolvedValue(status);
  vi.spyOn(api, "listEvents").mockResolvedValue([]);
  vi.spyOn(api, "listTasks").mockResolvedValue([]);
  vi.spyOn(api, "getModelSettings").mockResolvedValue(noModel);
  vi.spyOn(api, "getIntelligence").mockResolvedValue(intelligenceOf());
  vi.spyOn(api, "runTask").mockResolvedValue(null);
  vi.spyOn(api, "cancelTask").mockResolvedValue(null);
  vi.spyOn(api, "getHostCapabilities").mockResolvedValue({ platform: "macos", architecture: "arm64", backend: "real", backend_available: true, backend_detail: "", guest_transport: "virtio_socket", guest_transport_available: true, required_setup: [], supported: true });
  vi.spyOn(api, "suggestedEffects").mockResolvedValue({ tier: "reduced" });
  vi.spyOn(api, "accessibilityDisplay").mockResolvedValue({ reduce_transparency: false, increase_contrast: false });
  vi.spyOn(api, "captureScreen").mockRejectedValue(new Error("no screen in tests"));
  vi.spyOn(api, "getOnboarding").mockResolvedValue({ version: 1, step: "ready", completed: true, restart_requested: false });
});
afterEach(() => {
  cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.mocked(listen).mockClear();
  document.documentElement.removeAttribute("data-reduce-transparency");
  document.documentElement.removeAttribute("data-increase-contrast");
});

const composer = () => screen.getByRole("textbox", { name: "New task" }) as HTMLTextAreaElement;
const sidebar = () => screen.getByRole("complementary", { name: "Sidebar" });
const panel = () => screen.getByRole("complementary", { name: "Computer" });
const shell = () => document.querySelector(".shell");
const ready = async () => waitFor(() => expect(composer().disabled).toBe(false));
const machineRow = (state: RegExp) => within(sidebar()).findByRole("button", { name: state });

describe("desktop shell", () => {
  it("keeps a browser preview disconnected and never invokes the backend", () => {
    bridge.native = false;
    render(<App />);
    expect(composer().disabled).toBe(true);
    expect(api.getStatus).not.toHaveBeenCalled();
    fireEvent.click(within(sidebar()).getByRole("button", { name: /^Pegoles Computer: Offline/ }));
    expect(within(panel()).getByText("Computer unavailable")).toBeTruthy();
    expect(within(panel()).getByText(/Open the Pegoles app/)).toBeTruthy();
  });

  it("opens on a work surface: one question, the composer, and where the job will run", async () => {
    render(<App />);
    await ready();
    expect(screen.getByRole("heading", { level: 1, name: "Give Pegoles a task" })).toBeTruthy();
    expect(within(screen.getByRole("list", { name: "A few places to start" })).getAllByRole("button")).toHaveLength(3);
    // Local first: the way forward is setting up Pegoles Local, not a key.
    expect(screen.getByRole("button", { name: "Set up Pegoles Local. Open settings" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Runs on Pegoles Computer: Off/ })).toBeTruthy();
    // The computer is a status until asked for.
    expect(screen.queryByRole("complementary", { name: "Computer" })).toBeNull();
  });

  it("fills the composer from a starter without running anything", async () => {
    const create = vi.spyOn(api, "createTask");
    render(<App />);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Organize a folder" }));
    expect(composer().value).toMatch(/^Create a folder called notes/);
    expect(create).not.toHaveBeenCalled();
  });

  it("hands a task over and stays honest that it can't run yet", async () => {
    vi.spyOn(api, "createTask").mockImplementation(async () => {
      vi.mocked(api.listTasks).mockResolvedValue([task]);
      return task;
    });
    render(<App />);
    await ready();
    fireEvent.change(composer(), { target: { value: task.title } });
    fireEvent.submit(screen.getByRole("form", { name: "New task" }));
    expect(await screen.findByRole("heading", { level: 1, name: task.title })).toBeTruthy();
    expect(api.createTask).toHaveBeenCalledExactlyOnceWith(task.title);
    expect(await screen.findByText("Waiting for a model")).toBeTruthy();
    // Without a model nothing is started behind the person's back.
    expect(api.runTask).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
    await waitFor(() => expect(screen.getByRole("region", { name: "Now" }).textContent).toContain("Not started"));
    // No reply box on a task: there is no command for follow-ups.
    expect(screen.queryByRole("textbox", { name: "New task" })).toBeNull();
    await waitFor(() => expect(within(sidebar()).getByRole("button", { name: /Organize my notes, not started/ })).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: "Open its computer" }));
    expect(within(panel()).getByText("Pegoles will start its isolated computer when it needs it.")).toBeTruthy();
    // Focus and Full need a screen.
    expect(within(panel()).queryByRole("button", { name: "Focus its computer" })).toBeNull();
    fireEvent.click(within(panel()).getByRole("button", { name: "Close computer" }));
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Computer" })).toBeNull());
  });

  it("sets up Pegoles Local from a waiting task, shows its progress there, and offers Start once it's ready", async () => {
    vi.mocked(api.listTasks).mockResolvedValue([task]);
    const install = vi.spyOn(api, "installLocalModel").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 0) } }));
    render(<App />);
    await ready();
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /Organize my notes/ }));
    expect(await screen.findByText("Waiting for a model")).toBeTruthy();
    await waitFor(() => expect(screen.getByRole("region", { name: "Now" }).textContent).toContain("Pegoles Local isn’t set up yet"));

    fireEvent.click(await screen.findByRole("button", { name: "Set up Pegoles Local" }));
    await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith(MAI.id));
    expect(await screen.findByText("Setting up Pegoles Local")).toBeTruthy();
    emitInstall(installOf("downloading", 935_000_000));
    expect(screen.getByText("935 MB of 2.2 GB")).toBeTruthy();
    // Nothing starts on its own while it sets up.
    expect(api.runTask).not.toHaveBeenCalled();

    // Ready: Core says the chosen provider can run, and the task offers Start.
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" }, local: { install: installOf("ready", MAI.size_bytes) } }));
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, model: "configured" });
    emitInstall(installOf("ready", MAI.size_bytes));
    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    await waitFor(() => expect(api.runTask).toHaveBeenCalledExactlyOnceWith(task.id));
  });

  it("names Pegoles Local in the composer once it's ready", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, model: "configured" });
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" } }));
    // A stored cloud key doesn't matter while Pegoles Local is chosen.
    vi.mocked(api.getModelSettings).mockResolvedValue(withKey);
    render(<App />);
    await ready();
    expect(await screen.findByRole("button", { name: "Model: Pegoles Local" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Model: Pegoles Local" }));
    expect(await screen.findByRole("heading", { level: 2, name: "Intelligence" })).toBeTruthy();
    expect((screen.getByRole("radio", { name: "Pegoles Local" }) as HTMLInputElement).checked).toBe(true);
  });

  it("starts the job at once when a model is connected, and stops it with the task's own Stop", async () => {
    const connected = { ...status, model: "configured" as const, provider: "anthropic" as const };
    vi.mocked(api.getStatus).mockResolvedValue(connected);
    vi.mocked(api.getModelSettings).mockResolvedValue(withKey);
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ provider: "anthropic", anthropic: withKey }));
    vi.spyOn(api, "createTask").mockImplementation(async () => {
      vi.mocked(api.listTasks).mockResolvedValue([task]);
      return task;
    });
    vi.mocked(api.runTask).mockImplementation(async () => {
      vi.mocked(api.listTasks).mockResolvedValue([{ ...task, status: "running" }]);
      vi.mocked(api.getStatus).mockResolvedValue({ ...connected, active_task: task.id });
      vi.mocked(api.listEvents).mockResolvedValue([
        { type: "agent_message", task_id: task.id, kind: "progress", text: "Looking at the notes folder first", at: "2026-09-22T10:00:02Z" },
      ]);
      return null;
    });
    render(<App />);
    await ready();
    // The composer names the model Core will use: the cloud one, since it was chosen.
    expect(await screen.findByRole("button", { name: "Model: Claude Opus 5" })).toBeTruthy();
    fireEvent.change(composer(), { target: { value: task.title } });
    fireEvent.submit(screen.getByRole("form", { name: "New task" }));
    expect(await screen.findByRole("heading", { level: 1, name: task.title })).toBeTruthy();
    await waitFor(() => expect(api.runTask).toHaveBeenCalledExactlyOnceWith(task.id));
    const now = screen.getByRole("region", { name: "Now" });
    await waitFor(() => expect(now.textContent).toContain("Looking at the notes folder first"));
    expect(screen.getByRole("article", { name: "Note from Pegoles" }).textContent).toBe("Looking at the notes folder first");
    expect(screen.queryByText("Waiting for a model")).toBeNull();

    fireEvent.click(within(now).getByRole("button", { name: /Stop/ }));
    await waitFor(() => expect(api.cancelTask).toHaveBeenCalledExactlyOnceWith(task.id));
  });

  it("keeps a task whose start was refused, says why on the task, and offers Start again", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, model: "configured" });
    vi.mocked(api.listTasks).mockResolvedValue([task]);
    vi.mocked(api.runTask).mockRejectedValue("task 0199 is already running");
    render(<App />);
    await ready();
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /Organize my notes/ }));
    expect(await screen.findByText("Not started yet")).toBeTruthy();
    // Pending tasks never start on their own: only the person's Start does.
    expect(api.runTask).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(api.runTask).toHaveBeenCalledExactlyOnceWith(task.id));
    expect(await screen.findByText(/^Pegoles is working on another task\. It works on one task at a time/, { selector: ".notice__problem" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start" })).toBeTruthy();
  });

  it("keeps a failed hand-off's words and says so beside the prompt", async () => {
    vi.spyOn(api, "createTask").mockRejectedValue(new Error("Task storage unavailable"));
    render(<App />);
    await ready();
    fireEvent.change(composer(), { target: { value: task.title } });
    fireEvent.submit(screen.getByRole("form", { name: "New task" }));
    expect(await screen.findByText(/Couldn’t hand this to Pegoles/)).toBeTruthy();
    expect(composer().value).toBe(task.title);
    expect(screen.queryByRole("heading", { level: 1, name: task.title })).toBeNull();
  });

  it("routes human control through Core and never lets it be ambiguous", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, ...running, viewport_state: "user_controlled", control_owner: "user", display_attached: true });
    vi.spyOn(api, "returnControl").mockResolvedValue(undefined);
    render(<App />);
    fireEvent.click(await machineRow(/Pegoles Computer: You have control/));
    expect(within(panel()).getByText("You have control", { selector: ".strip__owner span" })).toBeTruthy();
    expect((within(panel()).getByRole("button", { name: "Close computer" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(within(panel()).getByRole("button", { name: "Give control to Pegoles" }));
    await waitFor(() => expect(api.returnControl).toHaveBeenCalledOnce());
  });

  it("moves one computer through Side, Focus and Full, and steps back with Escape", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, ...running });
    render(<App />);
    fireEvent.click(await machineRow(/Pegoles Computer: Ready/));
    expect(shell()?.getAttribute("data-computer")).toBe("side");
    const slot = document.querySelector("[data-framebuffer-slot]");
    expect(slot).toBeTruthy();
    fireEvent.click(within(panel()).getByRole("button", { name: "Focus its computer" }));
    expect(shell()?.getAttribute("data-computer")).toBe("focus");
    fireEvent.click(within(panel()).getByRole("button", { name: "Fill the window" }));
    expect(shell()?.getAttribute("data-computer")).toBe("full");
    expect(shell()?.getAttribute("data-sidebar")).toBe("hidden");
    // The same object at every level: the slot never remounts.
    expect(document.querySelector("[data-framebuffer-slot]")).toBe(slot);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(shell()?.getAttribute("data-computer")).toBe("focus");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(shell()?.getAttribute("data-computer")).toBe("side");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(shell()?.getAttribute("data-computer")).toBe("closed");
  });

  it("toggles its computer with ⌘J and returns focus to where it was", async () => {
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, ...running });
    render(<App />);
    await ready();
    const toggle = screen.getByRole("button", { name: /Show its computer \(Ready\)/ });
    toggle.focus();
    fireEvent.keyDown(window, { key: "j", metaKey: true });
    await waitFor(() => expect(document.activeElement).toBe(within(panel()).getByRole("heading", { name: "Computer" })));
    fireEvent.keyDown(window, { key: "j", metaKey: true });
    await waitFor(() => expect(document.activeElement).toBe(toggle));
  });

  it("shows a compact preview while Pegoles uses its computer, and opens it on request", async () => {
    const using: AgentTask = { ...task, status: "running" };
    const request = { action_id: "a1", task_id: using.id, computer_id: "vm", action: { type: "click", x: 0.2, y: 0.4 }, requested_at: "2026-09-22T10:00:05Z" };
    const events: AgentEvent[] = [{ type: "action_started", action_id: "a1", request, at: "2026-09-22T10:00:05Z" }];
    vi.mocked(api.getStatus).mockResolvedValue({ ...status, ...running, model: "configured", viewport_state: "agent_active", control_owner: "agent", agent_busy: true });
    vi.mocked(api.listTasks).mockResolvedValue([using]);
    vi.mocked(api.listEvents).mockResolvedValue(events);
    render(<App />);
    fireEvent.click(await within(sidebar()).findByRole("button", { name: /Organize my notes/ }));
    const watch = await screen.findByRole("button", { name: /^Watch its computer/ });
    expect(screen.queryByRole("complementary", { name: "Computer" })).toBeNull();
    expect(screen.getByRole("button", { name: /Stop/ })).toBeTruthy();
    fireEvent.click(watch);
    expect(await screen.findByRole("complementary", { name: "Computer" })).toBeTruthy();
    expect(within(panel()).getByText("Pegoles is using it")).toBeTruthy();
  });

  it("says a missing VM helper in human words and keeps the raw text in Details", async () => {
    const raw = "computer error: backend error: pegoles-vm-host binary not found (build the native helper or set PEGOLES_VM_HOST)";
    vi.spyOn(api, "createComputer").mockRejectedValue(raw);
    render(<App />);
    fireEvent.click(await machineRow(/Pegoles Computer: Off/));
    fireEvent.click(within(panel()).getByRole("button", { name: "Start computer" }));
    expect(await within(panel()).findByText("Computer couldn’t start.")).toBeTruthy();
    // The machine says it wherever it appears.
    expect(within(sidebar()).getByRole("button", { name: /Pegoles Computer: Can’t start/ })).toBeTruthy();
    expect(within(panel()).getByRole("button", { name: "Try again" })).toBeTruthy();
    const details = within(panel()).getByText("Details", { selector: ".computer__details summary" });
    expect(details.parentElement?.textContent).toContain("PEGOLES_VM_HOST");
    expect(screen.queryByText(raw, { selector: ".machine-state__headline, .machine-state__body" })).toBeNull();
  });

  it("persists motion quality without touching the system preference", async () => {
    render(<App />);
    await ready();
    fireEvent.click(within(sidebar()).getByRole("button", { name: "Settings" }));
    const group = await screen.findByRole("radiogroup", { name: "Motion quality" });
    expect(screen.getByRole("radio", { name: "Auto" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("radio", { name: "Reduced" }));
    expect(document.documentElement.dataset.effectsTier).toBe("reduced");
    expect(localStorage.getItem("pegoles.quality")).toBe("reduced");
    fireEvent.keyDown(group, { key: "ArrowLeft" });
    expect(screen.getByRole("radio", { name: "Full" }).getAttribute("aria-checked")).toBe("true");
    expect(localStorage.getItem("pegoles.quality")).toBe("full");
  });

  it("finds tasks and actions from the command palette (⌘K)", async () => {
    const many = Array.from({ length: 9 }, (_, i) => ({ ...task, id: `t${i}`, title: i === 4 ? "Rename photos" : `Task ${i}`, status: "completed" as const }));
    vi.mocked(api.listTasks).mockResolvedValue(many);
    render(<App />);
    await ready();
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Command palette" });
    const search = within(dialog).getByRole("combobox", { name: "Search tasks and actions" });
    fireEvent.change(search, { target: { value: "photos" } });
    expect(within(dialog).getAllByRole("option").map((option) => option.textContent)).toEqual([expect.stringContaining("Rename photos")]);
    fireEvent.keyDown(search, { key: "Enter" });
    expect(await screen.findByRole("heading", { level: 1, name: "Rename photos" })).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Command palette" })).toBeNull());

    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const again = within(await screen.findByRole("dialog", { name: "Command palette" })).getByRole("combobox");
    fireEvent.change(again, { target: { value: "zzz" } });
    expect(screen.getByText(/Nothing matches/)).toBeTruthy();
    fireEvent.keyDown(again, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Command palette" })).toBeNull());
  });

  it("groups tasks by what needs you, then by when they changed", async () => {
    const recent = new Date().toISOString();
    vi.mocked(api.listTasks).mockResolvedValue([
      { ...task, id: "a", title: "Waiting on you", status: "waiting_for_approval", updated_at: recent },
      { ...task, id: "b", title: "Finished today", status: "completed", updated_at: recent },
    ]);
    render(<App />);
    await ready();
    const tasks = within(sidebar()).getByRole("navigation", { name: "Tasks" });
    await waitFor(() => expect(within(tasks).getAllByRole("heading").map((heading) => heading.textContent)).toEqual(["Needs you", "Today"]));
    expect(screen.getByText("Pegoles needs your approval")).toBeTruthy();
  });

  it("starts a new task from anywhere with ⌘N", async () => {
    vi.mocked(api.listTasks).mockResolvedValue([task]);
    render(<App />);
    await ready();
    fireEvent.click(within(screen.getByRole("navigation", { name: "Tasks" })).getByRole("button", { name: /Organize my notes/ }));
    expect(await screen.findByRole("heading", { level: 1, name: "Organize my notes" })).toBeTruthy();
    fireEvent.keyDown(window, { key: "n", metaKey: true });
    expect(await screen.findByRole("heading", { level: 1, name: "Give Pegoles a task" })).toBeTruthy();
  });

  it("hides the sidebar and remembers it", async () => {
    render(<App />);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Hide sidebar" }));
    expect(shell()?.getAttribute("data-sidebar")).toBe("hidden");
    expect(localStorage.getItem("pegoles.sidebar")).toBe("hidden");
    fireEvent.click(screen.getByRole("button", { name: "Show sidebar" }));
    expect(shell()?.getAttribute("data-sidebar")).toBe("shown");
  });

  it("turns materials solid when macOS Reduce Transparency is on", async () => {
    vi.mocked(api.accessibilityDisplay).mockResolvedValue({ reduce_transparency: true, increase_contrast: false });
    render(<App />);
    await waitFor(() => expect(document.documentElement.hasAttribute("data-reduce-transparency")).toBe(true));
    vi.mocked(api.accessibilityDisplay).mockResolvedValue({ reduce_transparency: false, increase_contrast: true });
    fireEvent.focus(window);
    await waitFor(() => expect(document.documentElement.hasAttribute("data-reduce-transparency")).toBe(false));
    expect(document.documentElement.hasAttribute("data-increase-contrast")).toBe(true);
  });
});

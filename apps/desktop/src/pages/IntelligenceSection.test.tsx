import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IntelligenceSection } from "./IntelligenceSection";
import { useIntelligence } from "../state/useIntelligence";
import { useModelSettings } from "../state/useModelSettings";
import { installOf, intelligenceOf, MAI, NO_KEY, QWEN } from "../state/intelligenceFixture";
import { api, MODEL_INSTALL_EVENT, type ModelSettings } from "../lib/tauri";

const bus = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    bus.handlers.set(name, handler);
    return () => { bus.handlers.delete(name); };
  }),
}));

const stored: ModelSettings = { ...NO_KEY, configured: true, key_source: "keychain" };
const cloud = intelligenceOf({ provider: "anthropic" });

function Harness({ onChanged, native = true }: { onChanged?: () => void; native?: boolean }) {
  const intelligence = useIntelligence(native, onChanged);
  const model = useModelSettings(native, onChanged);
  return <IntelligenceSection id="settings-intelligence" native={native} intelligence={intelligence} model={model} />;
}

const emit = (payload: unknown) => act(() => { bus.handlers.get(MODEL_INSTALL_EVENT)?.({ payload }); });
const localRadio = () => screen.getByRole("radio", { name: "Pegoles Local" }) as HTMLInputElement;
const cloudRadio = () => screen.getByRole("radio", { name: "Anthropic" }) as HTMLInputElement;
const loaded = () => waitFor(() => expect(localRadio().disabled).toBe(false));
const keyButton = () => screen.getByRole("button", { name: /^(Add|Replace) key…$/ }) as HTMLButtonElement;
/** The page never has a field the key could be typed into (Core asks for it natively). */
const noKeyField = () => expect(document.querySelector("input[type=password], input[type=text], textarea")).toBeNull();

beforeEach(() => {
  vi.spyOn(api, "getIntelligence").mockResolvedValue(intelligenceOf());
  vi.spyOn(api, "getModelSettings").mockResolvedValue(NO_KEY);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); bus.handlers.clear(); });

describe("Intelligence settings: Pegoles Local", () => {
  it("offers Pegoles Local first, free and private, and sets it up in one step", async () => {
    const install = vi.spyOn(api, "installLocalModel").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 0) } }));
    render(<Harness />);
    await loaded();
    expect(localRadio().checked).toBe(true);
    expect(screen.getByText("Recommended")).toBeTruthy();
    expect(screen.getByText("Free · Private · Runs on this Mac")).toBeTruthy();
    expect(screen.getByText("Not set up")).toBeTruthy();
    expect(screen.getByText("One download of 2.2 GB, checked before it’s used. After that it works offline.")).toBeTruthy();
    // Nothing about the cloud is asked for.
    expect(screen.queryByRole("button", { name: "Add key…" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Set up" }));
    await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith(MAI.id));
    expect(await screen.findByText("Downloading model…")).toBeTruthy();
    expect(screen.getByText("Setting up…")).toBeTruthy();
  });

  it("shows real progress from Core's events, step by step, until it's ready", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 0) } }));
    render(<Harness />);
    await screen.findByText("Downloading model…");
    await waitFor(() => expect(bus.handlers.has(MODEL_INSTALL_EVENT)).toBe(true));

    emit(installOf("downloading", 935_000_000));
    expect(screen.getByText("935 MB of 2.2 GB")).toBeTruthy();
    expect(screen.getByText("41%")).toBeTruthy();
    const meter = screen.getByRole("progressbar", { name: "Setting up Pegoles Local" });
    expect(meter.getAttribute("aria-valuenow")).toBe("41");
    expect(meter.getAttribute("aria-valuetext")).toBe("935 MB of 2.2 GB");

    emit(installOf("verifying", MAI.size_bytes));
    expect(screen.getByText("Verifying…")).toBeTruthy();
    expect(screen.queryByText("41%")).toBeNull();
    emit(installOf("finalizing", MAI.size_bytes));
    expect(screen.getByText("Finishing…")).toBeTruthy();

    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" }, local: { install: installOf("ready", MAI.size_bytes) } }));
    emit(installOf("ready", MAI.size_bytes));
    expect(await screen.findByText("Ready")).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.queryByRole("button", { name: "Set up" })).toBeNull();
  });

  it("cancels a setup in progress", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 10) } }));
    const cancel = vi.spyOn(api, "cancelLocalModelInstall").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 10) } }));
    render(<Harness />);
    await screen.findByText("Downloading model…");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
  });

  it("says why a setup failed in Core's words, and tries again", async () => {
    const error = "Not enough disk space: Pegoles Local needs 2.4 GB free and 1.1 GB is available.";
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ local: { install: installOf("failed", 0, { error, error_kind: "disk_space" }) } }));
    const install = vi.spyOn(api, "installLocalModel").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 0) } }));
    render(<Harness />);
    expect((await screen.findByRole("alert")).textContent).toBe(error);
    expect(screen.getByText("Didn’t finish")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith(MAI.id));
  });

  it("resumes an interrupted download where it stopped", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({
      model: { state: "partial", partial_bytes: 823_000_000 },
      local: { install: installOf("failed", 0, { error: "The download was interrupted. Check your connection and retry; it resumes where it stopped.", error_kind: "network" }) },
    }));
    const install = vi.spyOn(api, "installLocalModel").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 823_000_000) } }));
    render(<Harness />);
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));
    await waitFor(() => expect(install).toHaveBeenCalledOnce());
  });

  it("says where a cancelled download paused", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "partial", partial_bytes: 823_000_000 }, local: { install: installOf("cancelled", 0) } }));
    render(<Harness />);
    expect(await screen.findByText("Paused at 823 MB of 2.2 GB. It picks up where it stopped.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("says when it's running locally, with the memory Core measured", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" }, local: { loaded_model: MAI.id, worker_footprint_bytes: 2_463_000_000 } }));
    render(<Harness />);
    expect(await screen.findByText("Running locally · 2.3 GB in memory")).toBeTruthy();
  });

  it("is honest when this Mac can't run it, and never offers a setup that can't work", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({
      local: { apple_silicon: false, runtime_ready: false, runtime_problem: "Pegoles Local needs a Mac with Apple silicon.", chip: null },
    }));
    render(<Harness />);
    expect(await screen.findByText("Pegoles Local needs a Mac with Apple silicon. You can still use a cloud model.")).toBeTruthy();
    expect(screen.getByText("Not available on this Mac")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Set up" })).toBeNull();
  });

  it("keeps the details under Advanced, exactly as Core reports them", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({
      model: { display_name: "Test Model X", parameters: "7B", quantization: "4bit", size_bytes: 3_100_000_000, license: "custom-license", source: "example.org/models/x @ 0123456789ab" },
    }));
    render(<Harness />);
    await loaded();
    const advanced = screen.getByText("Advanced").closest("details") as HTMLDetailsElement;
    expect(advanced.open).toBe(false);
    const rows = within(screen.getByRole("list", { name: "Pegoles Local model" }));
    const value = (label: string) => rows.getByText(label).closest(".setting")?.querySelector(".setting__value")?.textContent;
    expect(value("Model")).toBe("Test Model X");
    expect(value("Parameters")).toBe("7B");
    expect(value("Quantization")).toBe("4-bit");
    expect(value("Download size")).toBe("3.1 GB");
    expect(value("License")).toBe("custom-license");
    expect(value("Source")).toBe("example.org/models/x @ 0123456789ab");
    expect(value("This Mac")).toBe("Apple M3 Pro · 18 GB memory");
    // No recommendation is invented while Core has none.
    expect(rows.queryByText("Recommended memory")).toBeNull();
    // Nothing to remove before it's set up.
    expect(screen.queryByRole("button", { name: /Remove model/ })).toBeNull();
  });

  it("removes the model only after asking inline, with Cancel first", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" } }));
    const remove = vi.spyOn(api, "removeLocalModel").mockResolvedValue(intelligenceOf());
    render(<Harness />);
    await loaded();
    fireEvent.click(screen.getByRole("button", { name: "Remove model…" }));
    const ask = screen.getByRole("group", { name: "Remove MAI-UI 2B?" });
    expect(ask.textContent).toContain("Its 2.2 GB are deleted from this Mac.");
    expect(document.activeElement).toBe(within(ask).getByRole("button", { name: "Cancel" }));
    fireEvent.click(within(ask).getByRole("button", { name: "Cancel" }));
    expect(remove).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: "Remove MAI-UI 2B?" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Remove model…" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Remove MAI-UI 2B?" })).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(remove).toHaveBeenCalledExactlyOnceWith(MAI.id));
    expect(await screen.findByText("Not set up")).toBeTruthy();
  });

  it("says why a removal was refused", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "installed" } }));
    vi.spyOn(api, "removeLocalModel").mockRejectedValue("Stop the running task first.");
    render(<Harness />);
    await loaded();
    fireEvent.click(screen.getByRole("button", { name: "Remove model…" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Remove MAI-UI 2B?" })).getByRole("button", { name: "Remove" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Couldn’t remove the model. Stop the running task first.");
  });

  it("chooses which local model to use when there is more than one", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ models: [MAI, QWEN] }));
    const choose = vi.spyOn(api, "setProvider").mockResolvedValue(intelligenceOf({ models: [MAI, QWEN], local_model: QWEN.id }));
    render(<Harness />);
    await loaded();
    const qwen = screen.getByRole("radio", { name: "Qwen3-VL 2B Instruct, 4-bit" }) as HTMLInputElement;
    expect(qwen.checked).toBe(false);
    fireEvent.click(qwen);
    await waitFor(() => expect(choose).toHaveBeenCalledExactlyOnceWith("local", QWEN.id));
    await waitFor(() => expect((screen.getByRole("radio", { name: "Qwen3-VL 2B Instruct, 4-bit" }) as HTMLInputElement).checked).toBe(true));
  });
});

describe("Intelligence settings: choosing the cloud", () => {
  it("switches who plans through Core, and shows the key only once Anthropic is chosen", async () => {
    const onChanged = vi.fn();
    const choose = vi.spyOn(api, "setProvider").mockResolvedValue(cloud);
    render(<Harness onChanged={onChanged} />);
    await loaded();
    expect(screen.queryByRole("button", { name: "Add key…" })).toBeNull();
    expect(screen.getByText("No key")).toBeTruthy();
    fireEvent.click(cloudRadio());
    await waitFor(() => expect(choose).toHaveBeenCalledExactlyOnceWith("anthropic", undefined));
    await waitFor(() => expect(cloudRadio().checked).toBe(true));
    expect(localRadio().checked).toBe(false);
    expect(keyButton().textContent).toBe("Add key…");
    noKeyField();
    expect(screen.getByText(/screenshots of Pegoles’ computer \(its own virtual machine, never your Mac’s screen\)/)).toBeTruthy();
    expect(onChanged).toHaveBeenCalled();

    vi.mocked(api.setProvider).mockResolvedValue(intelligenceOf());
    fireEvent.click(localRadio());
    await waitFor(() => expect(choose).toHaveBeenLastCalledWith("local", undefined));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Add key…" })).toBeNull());
  });

  it("says Core's reason when the switch is refused, and keeps the choice as it was", async () => {
    vi.spyOn(api, "setProvider").mockRejectedValue("settings could not be saved");
    render(<Harness />);
    await loaded();
    fireEvent.click(cloudRadio());
    expect((await screen.findByRole("alert")).textContent).toBe("Couldn’t change the model. Settings could not be saved.");
    expect(localRadio().checked).toBe(true);
  });

  it("says in plain words when moving to the cloud wasn't confirmed in the macOS dialog", async () => {
    vi.spyOn(api, "setProvider").mockRejectedValue("Pegoles Local still plans your tasks: switching to Anthropic wasn't confirmed in the macOS dialog.");
    render(<Harness />);
    await loaded();
    fireEvent.click(cloudRadio());
    expect((await screen.findByRole("alert")).textContent)
      .toBe("Couldn’t change the model. Pegoles Local still plans your tasks: switching to Anthropic wasn't confirmed in the macOS dialog.");
    expect(localRadio().checked).toBe(true);
    expect(screen.queryByRole("button", { name: "Add key…" })).toBeNull();
  });

  it("has Core ask for the key in a macOS window, so the page never holds it", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(cloud);
    const onChanged = vi.fn();
    let answer: (value: ModelSettings) => void = () => undefined;
    const enter = vi.spyOn(api, "enterApiKey").mockReturnValue(new Promise((resolve) => { answer = resolve; }));
    render(<Harness onChanged={onChanged} />);
    await screen.findByText("Not connected");
    noKeyField();
    expect(screen.getByText(/You paste it in a macOS window Pegoles opens, never on this screen/)).toBeTruthy();
    fireEvent.click(keyButton());
    await waitFor(() => expect(enter).toHaveBeenCalledOnce());
    // Nothing is passed from the page: Core asks the person itself.
    expect(enter.mock.calls[0]).toEqual([]);
    expect(await screen.findByRole("button", { name: "Waiting for the key…" })).toBeTruthy();
    await act(async () => { answer(stored); });
    expect(await screen.findByText("Connected (Keychain)")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Key saved to your Keychain.");
    expect(keyButton().textContent).toBe("Replace key…");
    noKeyField();
    expect(onChanged).toHaveBeenCalledOnce();
  });

  it("shows Core's reason when no key was stored: cancelled, or refused", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(cloud);
    const enter = vi.spyOn(api, "enterApiKey").mockRejectedValue("the macOS key window was cancelled, so nothing was stored.");
    render(<Harness />);
    await screen.findByText("Not connected");
    fireEvent.click(keyButton());
    expect((await screen.findByRole("alert")).textContent).toBe("Couldn’t save the key. The macOS key window was cancelled, so nothing was stored.");
    expect(screen.queryByRole("status")).toBeNull();

    enter.mockRejectedValue("the key should be 20 to 256 characters");
    fireEvent.click(keyButton());
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Couldn’t save the key. The key should be 20 to 256 characters"));
    expect(screen.getByText("Not connected")).toBeTruthy();
  });

  it("removes a Keychain key, and names one from the environment without offering to remove it", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(cloud);
    vi.mocked(api.getModelSettings).mockResolvedValue(stored);
    const clear = vi.spyOn(api, "clearApiKey").mockResolvedValue(NO_KEY);
    const view = render(<Harness />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(clear).toHaveBeenCalledOnce());
    expect(await screen.findByText("Not connected")).toBeTruthy();
    view.unmount();

    vi.mocked(api.getModelSettings).mockResolvedValue({ ...stored, key_source: "environment" });
    render(<Harness />);
    expect(await screen.findByText("From ANTHROPIC_API_KEY")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove key" })).toBeNull();
    expect(keyButton().textContent).toBe("Replace key…");
  });

  it("changes the cloud model and effort through Core", async () => {
    vi.mocked(api.getIntelligence).mockResolvedValue(cloud);
    vi.mocked(api.getModelSettings).mockResolvedValue(stored);
    const choose = vi.spyOn(api, "setModelSettings").mockImplementation(async (model, effort) => ({ ...stored, model, effort }));
    render(<Harness />);
    const model = await screen.findByRole("combobox", { name: "Model" }) as HTMLSelectElement;
    expect([...model.options].map((option) => option.textContent)).toEqual(["Claude Opus 5", "Claude Sonnet 5", "Claude Opus 5.5"]);
    await act(async () => { fireEvent.change(model, { target: { value: "claude-opus-5-5" } }); });
    expect(choose).toHaveBeenLastCalledWith("claude-opus-5-5", "high");
    const effort = screen.getByRole("combobox", { name: "Effort" }) as HTMLSelectElement;
    expect([...effort.options].map((option) => option.textContent)).toContain("Extra high");
    await act(async () => { fireEvent.change(effort, { target: { value: "max" } }); });
    expect(choose).toHaveBeenLastCalledWith("claude-opus-5-5", "max");
  });

  it("needs the desktop app, and never asks Core from a browser preview", () => {
    render(<Harness native={false} />);
    expect(screen.getAllByText("Desktop app required")).toHaveLength(2);
    expect(api.getIntelligence).not.toHaveBeenCalled();
    expect(api.getModelSettings).not.toHaveBeenCalled();
    expect(screen.queryByRole("radio")).toBeNull();
  });
});

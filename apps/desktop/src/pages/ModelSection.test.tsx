import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModelSection } from "./ModelSection";
import { useModelSettings } from "../state/useModelSettings";
import { api, type ModelSettings } from "../lib/tauri";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));

const KEY = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
const none: ModelSettings = {
  configured: false, key_source: null, model: "claude-opus-5", effort: "high",
  models: ["claude-opus-5", "claude-sonnet-5", "claude-opus-5-5"], efforts: ["low", "medium", "high", "xhigh", "max"],
};
const stored: ModelSettings = { ...none, configured: true, key_source: "keychain" };

function Harness({ onChanged, native = true }: { onChanged?: () => void; native?: boolean }) {
  const model = useModelSettings(native, onChanged);
  return <ModelSection id="settings-model" native={native} model={model} />;
}

const field = () => screen.getByLabelText(/Add a key|Replace the key/) as HTMLInputElement;
const type = (value: string) => {
  const input = field();
  input.value = value;
  fireEvent.input(input);
};

beforeEach(() => {
  vi.spyOn(api, "getModelSettings").mockResolvedValue(none);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("Model settings", () => {
  it("says honestly that no key is connected, and what running a task sends where", async () => {
    render(<Harness />);
    expect(await screen.findByText("Not connected")).toBeTruthy();
    expect(screen.getByText(/screenshots of Pegoles’ computer \(its own virtual machine, never your Mac’s screen\)/)).toBeTruthy();
    expect(screen.getByText(/key stays in your Mac’s Keychain and never reaches Pegoles’ computer/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove key" })).toBeNull();
  });

  it("stores a key in the Keychain and never shows it again", async () => {
    const onChanged = vi.fn();
    const save = vi.spyOn(api, "setApiKey").mockResolvedValue(stored);
    render(<Harness onChanged={onChanged} />);
    await screen.findByText("Not connected");
    expect(field().type).toBe("password");
    const submit = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    type(KEY);
    expect(submit.disabled).toBe(false);
    fireEvent.click(submit);
    await waitFor(() => expect(save).toHaveBeenCalledExactlyOnceWith(KEY));
    expect(await screen.findByText("Connected (Keychain)")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Key saved to your Keychain.");
    expect(field().value).toBe("");
    expect(document.body.innerHTML).not.toContain(KEY);
    expect(onChanged).toHaveBeenCalledOnce();
  });

  it("shows Core's reason when a key is refused, and keeps what was typed to fix it", async () => {
    vi.spyOn(api, "setApiKey").mockRejectedValue("the key should be 20 to 256 characters");
    render(<Harness />);
    await screen.findByText("Not connected");
    type("sk-short");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Couldn’t save the key. The key should be 20 to 256 characters");
    expect(field().value).toBe("sk-short");
  });

  it("removes a Keychain key", async () => {
    vi.mocked(api.getModelSettings).mockResolvedValue(stored);
    const clear = vi.spyOn(api, "clearApiKey").mockResolvedValue(none);
    render(<Harness />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(clear).toHaveBeenCalledOnce());
    expect(await screen.findByText("Not connected")).toBeTruthy();
  });

  it("names a key from the environment and doesn't offer to remove it", async () => {
    vi.mocked(api.getModelSettings).mockResolvedValue({ ...stored, key_source: "environment" });
    render(<Harness />);
    expect(await screen.findByText("From ANTHROPIC_API_KEY")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove key" })).toBeNull();
    expect(screen.getByLabelText(/Replace the key/)).toBeTruthy();
  });

  it("changes the model and effort through Core", async () => {
    const choose = vi.spyOn(api, "setModelSettings").mockImplementation(async (model, effort) => ({ ...stored, model, effort }));
    vi.mocked(api.getModelSettings).mockResolvedValue(stored);
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
    expect(screen.getByText("Desktop app required")).toBeTruthy();
    expect(api.getModelSettings).not.toHaveBeenCalled();
    expect(screen.queryByLabelText(/Add a key/)).toBeNull();
  });
});

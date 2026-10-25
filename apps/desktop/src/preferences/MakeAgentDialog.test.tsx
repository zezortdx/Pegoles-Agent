import { useState } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FluxGlassRoot } from "@pegoles/ui";
import { api } from "../lib/tauri";
import { PreferencesProvider } from "./PreferencesProvider";
import { MakeAgentDialog } from "./MakeAgentDialog";
import { DEFAULT_PREFERENCES, type Preferences } from "./types";
import { PresenceAvatar } from "../presence/PresenceAvatar";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));
let persisted: Preferences;
beforeEach(() => {
  persisted = DEFAULT_PREFERENCES;
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: query.includes("prefers-reduced-motion"), addEventListener() {}, removeEventListener() {} }));
  vi.spyOn(api, "getPreferences").mockImplementation(async () => persisted);
  vi.spyOn(api, "setPreferences").mockImplementation(async (preferences) => {
    persisted = { ...preferences, avatar: { ...preferences.avatar, name: preferences.avatar.name.trim() } };
    return persisted;
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function Harness() {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("user <b>original</b>");
  return <PreferencesProvider><FluxGlassRoot tier="reduced" reducedMotion>
    <textarea aria-label="Task draft" value={draft} onChange={(e) => setDraft(e.target.value)} />
    <PresenceAvatar mode="idle" size={80} />
    <button onClick={() => setOpen(true)}>Customize</button>
    {open && <MakeAgentDialog onClose={() => setOpen(false)} />}
  </FluxGlassRoot></PreferencesProvider>;
}
const open = async (waitUntilReady = true) => {
  const trigger = screen.getByRole("button", { name: "Customize" });
  trigger.focus(); fireEvent.click(trigger);
  const dialog = await screen.findByRole("dialog", { name: "Make your agent" });
  if (waitUntilReady) await waitFor(() => expect((within(dialog).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false));
  return dialog;
};
describe("agent preferences", () => {
  it("waits for Core preferences before accepting edits to the initial default", async () => {
    let loaded!: (preferences: Preferences) => void;
    vi.mocked(api.getPreferences).mockImplementation(() => new Promise((resolve) => { loaded = resolve; }));
    render(<Harness />);
    const dialog = await open(false);
    expect((within(dialog).getByRole("textbox", { name: "Agent name" }) as HTMLInputElement).disabled).toBe(true);
    expect((within(dialog).getByRole("combobox", { name: "Language" }) as HTMLSelectElement).disabled).toBe(true);
    expect((within(dialog).getByRole("radio", { name: "Cloud" }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => loaded({ ...DEFAULT_PREFERENCES, avatar: { ...DEFAULT_PREFERENCES.avatar, name: "Saved agent" } }));
    await waitFor(() => expect((within(dialog).getByRole("textbox", { name: "Agent name" }) as HTMLInputElement).disabled).toBe(false));
    expect((within(dialog).getByRole("textbox", { name: "Agent name" }) as HTMLInputElement).value).toBe("Saved agent");
  });
  it("loads, edits, confirms and reopens the profile, and keeps preview motion static", async () => {
    render(<Harness />);
    const dialog = await open();
    const name = within(dialog).getByRole("textbox", { name: "Agent name" });
    expect(document.activeElement).toBe(name);
    fireEvent.change(name, { target: { value: "  My <agent> 🪩  " } });
    fireEvent.click(within(dialog).getByRole("radio", { name: "Cloud" }));
    fireEvent.click(within(dialog).getByRole("radio", { name: "Graphite" }));
    fireEvent.click(within(dialog).getByRole("radio", { name: "Curious" }));
    expect(dialog.querySelector("svg[data-frame='static'][data-shape='cloud']")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Customize" }));
    expect(document.querySelector("svg[data-shape='cloud'][data-color='graphite']")).toBeTruthy();
    expect(screen.getByRole("img", { name: "My <agent> 🪩 is idle" })).toBeTruthy();
    const reopened = await open();
    expect((within(reopened).getByRole("textbox", { name: "Agent name" }) as HTMLInputElement).value).toBe("My <agent> 🪩");
    cleanup(); render(<Harness />);
    const afterReload = await open();
    expect((within(afterReload).getByRole("textbox", { name: "Agent name" }) as HTMLInputElement).value).toBe("My <agent> 🪩");
  });
  it("persists pt-BR only after Core confirms and preserves the original task draft", async () => {
    let confirm!: (preferences: Preferences) => void;
    vi.mocked(api.setPreferences).mockImplementation((preferences) => new Promise((resolve) => { confirm = () => { persisted = preferences; resolve(preferences); }; }));
    render(<Harness />); const dialog = await open();
    fireEvent.change(within(dialog).getByRole("combobox", { name: "Language" }), { target: { value: "pt-BR" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(document.documentElement.lang).toBe("en");
    expect((within(dialog).getByRole("textbox") as HTMLInputElement).disabled).toBe(true);
    fireEvent.keyDown(dialog, { key: "Escape" }); expect(screen.getByRole("dialog")).toBeTruthy();
    await act(async () => confirm({ ...DEFAULT_PREFERENCES, locale: "pt-BR" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.documentElement.lang).toBe("pt-BR");
    expect((screen.getByRole("textbox", { name: "Task draft" }) as HTMLTextAreaElement).value).toBe("user <b>original</b>");
    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    expect(await screen.findByRole("dialog", { name: "Monte seu agente" })).toBeTruthy();
    cleanup(); render(<Harness />);
    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    expect(await screen.findByRole("dialog", { name: "Monte seu agente" })).toBeTruthy();
  });
  it("contains keyboard focus, supports arrows, and restores focus after save failure and cancel", async () => {
    vi.mocked(api.setPreferences).mockRejectedValue(new Error("disk full"));
    render(<Harness />); const dialog = await open();
    const circle = within(dialog).getByRole("radio", { name: "Circle" });
    circle.focus(); fireEvent.keyDown(circle, { key: "ArrowRight" });
    expect(document.activeElement).toBe(within(dialog).getByRole("radio", { name: "Pebble" }));
    const save = within(dialog).getByRole("button", { name: "Save" });
    save.focus(); fireEvent.keyDown(save, { key: "Tab" });
    expect(document.activeElement).toBe(within(dialog).getByRole("textbox", { name: "Agent name" }));
    fireEvent.click(save);
    expect(await within(dialog).findByRole("alert")).toHaveProperty("textContent", "Could not save your agent. Try again.");
    expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Save" }));
    expect(document.querySelector("svg[data-shape='circle']")).toBeTruthy();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Customize" }));
  });
  it("reports load errors and requires retry before replacing unreadable preferences", async () => {
    vi.mocked(api.getPreferences).mockRejectedValue(new Error("corrupt"));
    render(<Harness />); const dialog = await open(false);
    expect(await within(dialog).findByRole("alert")).toHaveProperty("textContent", "Could not load preferences. Try again.");
    expect((within(dialog).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    vi.mocked(api.getPreferences).mockResolvedValue(DEFAULT_PREFERENCES);
    fireEvent.click(within(dialog).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect((within(dialog).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false));
  });
});

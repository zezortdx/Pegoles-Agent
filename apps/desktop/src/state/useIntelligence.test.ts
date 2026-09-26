import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { intelligenceProblem, isInstallStatus, useIntelligence } from "./useIntelligence";
import { installOf, intelligenceOf, MAI } from "./intelligenceFixture";
import { api, MODEL_INSTALL_EVENT, type Intelligence } from "../lib/tauri";

const bus = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    bus.handlers.set(name, handler);
    return () => { bus.handlers.delete(name); };
  }),
}));

const emit = (payload: unknown) => act(() => { bus.handlers.get(MODEL_INSTALL_EVENT)?.({ payload }); });
const listening = () => waitFor(() => expect(bus.handlers.has(MODEL_INSTALL_EVENT)).toBe(true));

beforeEach(() => {
  vi.spyOn(api, "getIntelligence").mockResolvedValue(intelligenceOf());
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); bus.handlers.clear(); });

describe("useIntelligence", () => {
  it("never asks Core from a browser preview", () => {
    const { result } = renderHook(() => useIntelligence(false));
    expect(result.current.intelligence).toBeNull();
    expect(api.getIntelligence).not.toHaveBeenCalled();
    expect(bus.handlers.size).toBe(0);
  });

  it("follows setup progress from events, and reads Core again once it settles", async () => {
    const onChanged = vi.fn();
    const { result } = renderHook(() => useIntelligence(true, onChanged));
    await waitFor(() => expect(result.current.intelligence).not.toBeNull());
    await listening();

    emit(installOf("downloading", 935_000_000));
    expect(result.current.intelligence?.local.install).toMatchObject({ phase: "downloading", done_bytes: 935_000_000 });
    expect(onChanged).not.toHaveBeenCalled();

    const installed = intelligenceOf({ model: { state: "installed" }, local: { install: installOf("ready", MAI.size_bytes) } });
    vi.mocked(api.getIntelligence).mockResolvedValue(installed);
    const reads = vi.mocked(api.getIntelligence).mock.calls.length;
    emit(installOf("ready", MAI.size_bytes));
    await waitFor(() => expect(result.current.intelligence?.local.models[0].state).toBe("installed"));
    expect(vi.mocked(api.getIntelligence).mock.calls.length).toBe(reads + 1);
    // Core's status (can a task start now?) catches up too.
    expect(onChanged).toHaveBeenCalledOnce();
  });

  it("ignores payloads that aren't a setup status", async () => {
    const { result } = renderHook(() => useIntelligence(true));
    await waitFor(() => expect(result.current.intelligence).not.toBeNull());
    await listening();
    emit({ model: MAI.id, phase: "exploding", done_bytes: 1, total_bytes: 2, error: null, error_kind: null });
    emit({ model: MAI.id, phase: "downloading", done_bytes: -1, total_bytes: 2, error: null, error_kind: null });
    emit("<b>hi</b>");
    expect(result.current.intelligence?.local.install).toBeNull();
    expect(isInstallStatus(installOf("failed", 0, { error: "x", error_kind: "network" }))).toBe(true);
  });

  it("keeps a newer event over the snapshot of a command sent before it", async () => {
    const { result } = renderHook(() => useIntelligence(true));
    await waitFor(() => expect(result.current.intelligence).not.toBeNull());
    await listening();
    // Core answers the cancel before its setup thread has stopped...
    let answer: (value: Intelligence) => void = () => undefined;
    vi.spyOn(api, "cancelLocalModelInstall").mockReturnValue(new Promise((resolve) => { answer = resolve; }));
    let cancelled: Promise<boolean> = Promise.resolve(false);
    act(() => { cancelled = result.current.cancelInstall(); });
    expect(result.current.pending).toBe("cancel");
    // ...the thread says so first...
    vi.mocked(api.getIntelligence).mockResolvedValue(intelligenceOf({ model: { state: "partial", partial_bytes: 400 }, local: { install: installOf("cancelled", 0) } }));
    emit(installOf("cancelled", 0));
    // ...and the older snapshot arrives last: it doesn't undo the cancel.
    await act(async () => { answer(intelligenceOf({ local: { install: installOf("downloading", 300) } })); await cancelled; });
    await waitFor(() => expect(result.current.intelligence?.local.install?.phase).toBe("cancelled"));
    expect(result.current.pending).toBeNull();
  });

  it("keeps Core's words when an operation is refused, and one at a time", async () => {
    vi.spyOn(api, "removeLocalModel").mockRejectedValue("Stop the running task first.");
    const { result } = renderHook(() => useIntelligence(true));
    await waitFor(() => expect(result.current.intelligence).not.toBeNull());
    let ok = true;
    await act(async () => { ok = await result.current.remove(MAI.id); });
    expect(ok).toBe(false);
    expect(result.current.error).toEqual({ op: "remove", text: "Stop the running task first." });
    expect(intelligenceProblem(result.current.error)).toBe("Couldn’t remove the model. Stop the running task first.");
    expect(intelligenceProblem(result.current.error, ["install"])).toBeNull();
  });

  it("repairs damaged files by removing them, then setting the model up again", async () => {
    const remove = vi.spyOn(api, "removeLocalModel").mockResolvedValue(intelligenceOf());
    const install = vi.spyOn(api, "installLocalModel").mockResolvedValue(intelligenceOf({ local: { install: installOf("downloading", 0) } }));
    const { result } = renderHook(() => useIntelligence(true));
    await waitFor(() => expect(result.current.intelligence).not.toBeNull());
    await act(async () => { await result.current.repair(MAI.id); });
    expect(remove).toHaveBeenCalledExactlyOnceWith(MAI.id);
    expect(install).toHaveBeenCalledExactlyOnceWith(MAI.id);
    expect(result.current.intelligence?.local.install?.phase).toBe("downloading");
  });
});

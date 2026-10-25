import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_PREFERENCES } from "../preferences/types";
import { describe, expect, it, vi } from "vitest";
import { api, debugApi } from "../lib/tauri";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * The release web view is granted exactly the commands in `api`
 * (src-tauri/capabilities/default.json, held equal to `api` by the desktop
 * crate's tests). These keep `api` to what the product really calls, and
 * the Design Lab's `debugApi` out of the product.
 */
const product = import.meta.glob<string>(
  ["../**/*.{ts,tsx}", "!../**/*.test.{ts,tsx}", "!../dev/**", "!../lib/tauri.ts", "!../state/intelligenceFixture.ts"],
  { query: "?raw", import: "default", eager: true },
);

describe("IPC surface of the product", () => {
  it("history reads accept only a task identifier with no host path or URL", async () => {
    vi.mocked(invoke).mockClear(); await api.getHistoryStatus(); await api.getTaskHistory("task-id");
    expect(vi.mocked(invoke).mock.calls).toEqual([["get_history_status"],["get_task_history", {taskId:"task-id"}]]);
  });

  it("preferences expose only fixed get/set payloads, without paths or styling parameters", async () => {
    vi.mocked(invoke).mockClear();
    await api.getPreferences();
    await api.setPreferences(DEFAULT_PREFERENCES);
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["get_preferences"],
      ["set_preferences", { preferences: DEFAULT_PREFERENCES }],
    ]);
    expect(Object.keys(DEFAULT_PREFERENCES).sort()).toEqual(["avatar", "locale", "schema_version"]);
    expect(Object.keys(DEFAULT_PREFERENCES.avatar).sort()).toEqual(["color", "expression", "name", "shape"]);
  });

  it("every command the release bridge exposes is called by the product", () => {
    const sources = Object.values(product);
    expect(sources.length).toBeGreaterThan(20);
    const unused = Object.keys(api).filter((name) => !sources.some((source) => new RegExp(`\\bapi\\.${name}\\b`).test(source)));
    expect(unused).toEqual([]);
  });

  it("the product never calls debug-only commands", () => {
    expect(Object.keys(debugApi).length).toBeGreaterThan(0);
    const offenders = Object.entries(product).filter(([, source]) => /\bdebugApi\b/.test(source)).map(([file]) => file);
    expect(offenders).toEqual([]);
  });
});

import { describe, expect, it } from "vitest";
import { api, debugApi } from "../lib/tauri";

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

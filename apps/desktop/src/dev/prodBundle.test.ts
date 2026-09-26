import { describe, expect, it } from "vitest";

// Built assets, read as text at transform time. Empty when no build exists.
const built = import.meta.glob<string>(["../../dist/**/*.js", "../../dist/**/*.html", "../../dist/**/*.css"], {
  query: "?raw",
  import: "default",
  eager: true,
});

// The IPC command lists Core registers (release vs debug-only).
const ipcCommands = Object.values(import.meta.glob<string>("../../src-tauri/src/ipc_commands.rs", {
  query: "?raw",
  import: "default",
  eager: true,
}))[0] ?? "";

/**
 * Set by scripts/check.sh for the step right after `pnpm build`: the bundle
 * just built must exist and is checked. The plain unit run skips these
 * checks, since whatever dist/ holds then may be stale or missing.
 */
const REQUIRED = import.meta.env.PEGOLES_REQUIRE_BUNDLE === "1";

// Split so this test file itself never contains the literal markers.
const marker = (name: string) => ["__PEGOLES", name, "LAB__"].join("_");
const MARKERS = [marker("DESIGN"), marker("EXPERIENCE"), marker("SHELL")];

function commandList(section: "release" | "debug"): string[] {
  const body = new RegExp(`${section}:\\s*\\[([^\\]]*)\\]`).exec(ipcCommands)?.[1] ?? "";
  return body.split(",").map((name) => name.trim()).filter(Boolean);
}

/** Text a person using the product must never be told: repository scripts and docs. */
const DEVELOPER_TEXT = ["build-guest-image", "setup-runtime", "seal_image", "PROJECT_STATE", "cargo run"];

const quoted = (source: string, name: string) => [`"${name}"`, `'${name}'`, `\`${name}\``].some((form) => source.includes(form));
const offenders = (found: (source: string) => boolean) =>
  Object.entries(built).filter(([, source]) => found(source)).map(([file]) => file);

describe("production bundle", () => {
  it.runIf(REQUIRED)("exists: the check runs on the bundle just built", () => {
    expect(Object.keys(built).some((file) => file.endsWith("/index.html"))).toBe(true);
    expect(Object.keys(built).some((file) => file.endsWith(".js"))).toBe(true);
  });

  it.runIf(REQUIRED)("does not contain the dev-only labs", () => {
    expect(offenders((source) => MARKERS.some((m) => source.includes(m)))).toEqual([]);
  });

  it.runIf(REQUIRED)("calls release commands only: no debug-only or removed command names", () => {
    const debug = commandList("debug");
    expect(debug).toContain("execute_action");
    expect(commandList("release")).toContain("get_status");
    for (const name of [...debug, "set_api_key"]) {
      expect(offenders((source) => quoted(source, name)), name).toEqual([]);
    }
    // Sanity: this is the real app bundle, and it does reach Core.
    expect(offenders((source) => quoted(source, "get_status"))).not.toEqual([]);
  });

  it.runIf(REQUIRED)("never tells people to run developer scripts", () => {
    for (const text of DEVELOPER_TEXT) {
      expect(offenders((source) => source.includes(text)), text).toEqual([]);
    }
  });

  it("the markers match the lab constants", async () => {
    const { DESIGN_LAB_MARKER } = await import("./DesignLab");
    const { EXPERIENCE_LAB_MARKER } = await import("./ExperienceLab");
    const { SHELL_LAB_MARKER } = await import("./shellLab");
    expect([DESIGN_LAB_MARKER, EXPERIENCE_LAB_MARKER, SHELL_LAB_MARKER]).toEqual(MARKERS);
  }, 20_000);
});

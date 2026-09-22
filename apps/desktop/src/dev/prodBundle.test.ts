import { describe, expect, it } from "vitest";

// Built assets, read as text at transform time. Empty when no build exists.
const built = import.meta.glob<string>(["../../dist/**/*.js", "../../dist/**/*.html", "../../dist/**/*.css"], {
  query: "?raw",
  import: "default",
  eager: true,
});

// Kept in sync with DESIGN_LAB_MARKER in DesignLab.tsx. Split so this
// test file itself never contains the literal marker.
const MARKER = ["__PEGOLES", "DESIGN", "LAB__"].join("_");

describe("production bundle", () => {
  it.skipIf(Object.keys(built).length === 0)(
    "does not contain the dev-only design lab (run after `pnpm --filter @pegoles/desktop build`)",
    () => {
      const offenders = Object.entries(built)
        .filter(([, source]) => source.includes(MARKER))
        .map(([file]) => file);
      expect(offenders).toEqual([]);
    },
  );

  it("the marker matches the design lab constant", async () => {
    const { DESIGN_LAB_MARKER } = await import("./DesignLab");
    expect(DESIGN_LAB_MARKER).toBe(MARKER);
  });
});

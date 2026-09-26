import { describe, expect, it } from "vitest";
import { relativeTime } from "./taskPresentation";

describe("task presentation", () => {
  const now = new Date(2026, 8, 24, 15, 0, 0).getTime();

  it("formats relative time from just now to a short date", () => {
    expect(relativeTime(new Date(now - 20_000).toISOString(), now)).toBe("Just now");
    expect(relativeTime(new Date(now - 12 * 60_000).toISOString(), now)).toBe("12 min ago");
    expect(relativeTime(new Date(now - 3 * 3_600_000).toISOString(), now)).toBe("3 h ago");
    expect(relativeTime(new Date(2026, 8, 23, 22, 0).toISOString(), now)).toBe("Yesterday");
    expect(relativeTime(new Date(2026, 8, 20, 9, 0).toISOString(), now)).not.toMatch(/ago|Yesterday/);
    expect(relativeTime("not a date", now)).toBe("");
  });
});

import { hostLabel, sentenceCase } from "./format";

describe("format", () => {
  it("labels hosts and wire words for people", () => {
    expect(hostLabel("macos", "arm64")).toBe("macOS · Apple silicon");
    expect(hostLabel("windows", "x86_64")).toBe("Windows · x64");
    expect(hostLabel("plan9", "mips")).toBe("plan9 · mips");
    expect(sentenceCase("not_configured")).toBe("Not configured");
    expect(sentenceCase("")).toBe("");
  });
});

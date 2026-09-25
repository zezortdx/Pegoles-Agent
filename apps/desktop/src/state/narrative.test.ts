import { describe, expect, it } from "vitest";
import { pinnedSteps, runSentence, runSummary } from "./narrative";
import type { TranscriptStep } from "./transcript";

const step = (id: string, patch: Partial<TranscriptStep> = {}): TranscriptStep => ({
  id, label: id, outcome: "done", at: "2026-09-26T10:00:00Z", verb: "click", capability: "Computer", consequential: false, durationMs: 100, ...patch,
});

describe("runSummary", () => {
  it("says what a run did as one sentence of distinct steps", () => {
    expect(runSummary([step("Looked at the screen"), step("Clicked"), step("Clicked"), step("Typed text")]))
      .toEqual({ label: "Looked at the screen, clicked and typed text", count: 4, durationMs: 400, capability: "Computer" });
  });

  it("keeps the sentence short when a run did many kinds of things", () => {
    expect(runSentence([step("A"), step("B"), step("C"), step("D")])).toBe("A, b, c and more");
    expect(runSentence([step("Read a file")])).toBe("Read a file");
  });

  it("omits the duration unless Core measured every step", () => {
    expect(runSummary([step("a"), step("b", { durationMs: undefined })]).durationMs).toBeUndefined();
  });
});

describe("pinnedSteps", () => {
  it("keeps consequential and unsuccessful steps visible", () => {
    const steps = [step("look"), step("write", { consequential: true }), step("denied", { outcome: "blocked" })];
    expect(pinnedSteps(steps).map((item) => item.id)).toEqual(["write", "denied"]);
  });
});

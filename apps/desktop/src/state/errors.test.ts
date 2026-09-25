import { describe, expect, it } from "vitest";
import { errorText, humanizeError } from "./errors";

describe("humanizeError", () => {
  it("turns a missing VM helper into one calm sentence and keeps the raw text for Details", () => {
    const raw = "computer error: backend error: pegoles-vm-host binary not found (build the native helper or set PEGOLES_VM_HOST)";
    const error = humanizeError(raw, "general");
    expect(error).toMatchObject({ scope: "computer", title: "Computer couldn’t start.", retryable: true });
    expect(error.title).not.toMatch(/binary|PEGOLES_VM_HOST/);
    expect(error.detail).toBe(raw);
  });

  it("keeps a failed hand-off in the task scope with a reassuring hint", () => {
    expect(humanizeError(new Error("Task storage unavailable"), "task")).toMatchObject({
      scope: "task", title: "Couldn’t hand this to Pegoles.", hint: "Your text is still here.", detail: "Task storage unavailable",
    });
  });

  it("never loses the detail", () => {
    expect(humanizeError("", "general").detail).toBe("No details were reported.");
    expect(errorText({ code: 7 })).toBe('{"code":7}');
  });
});

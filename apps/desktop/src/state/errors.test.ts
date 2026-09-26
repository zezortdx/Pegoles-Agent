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

  it("keeps a failed start with its task, in words people can act on", () => {
    expect(humanizeError("Connect a model in Settings first.", "run")).toMatchObject({ scope: "run", title: "Connect a model first." });
    expect(humanizeError("task 0191 is already running", "run")).toMatchObject({ scope: "run", title: "Pegoles is working on another task." });
    expect(humanizeError("task is Running, not pending", "run")).toMatchObject({ scope: "run", title: "This task has already started." });
    // A computer failure while starting still reads as a computer problem, but stays with the task.
    const helper = humanizeError("computer error: backend error: pegoles-vm-host binary not found", "run");
    expect(helper).toMatchObject({ scope: "run", title: "Computer couldn’t start." });
    expect(humanizeError("something odd", "run")).toMatchObject({ scope: "run", title: "Pegoles couldn’t start this task." });
  });

  it("explains a refused reset or removal, and a missing computer image", () => {
    expect(humanizeError("stop the running task before resetting the computer", "general")).toMatchObject({ scope: "general", title: "Stop the running task first." });
    expect(humanizeError("computer image missing: no sealed image", "computer")).toMatchObject({ title: "Pegoles’ computer image isn’t ready." });
  });

  it("never loses the detail", () => {
    expect(humanizeError("", "general").detail).toBe("No details were reported.");
    expect(errorText({ code: 7 })).toBe('{"code":7}');
  });
});

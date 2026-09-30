import { describe, expect, it } from "vitest";
import { errorText, humanizeError, stopReason } from "./errors";

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

  it("says what a task needs to start, local first", () => {
    expect(humanizeError("Set up Pegoles Local in Settings first.", "run")).toMatchObject({
      scope: "run", title: "Pegoles Local isn’t set up yet.", hint: "Set it up in Settings (free, runs on this Mac), or connect a cloud model.",
    });
    expect(humanizeError("Connect a model in Settings first: add an Anthropic key or switch to Pegoles Local.", "run")).toMatchObject({
      scope: "run", title: "Cloud mode needs an Anthropic key.", hint: "Add one in Settings, or switch to Pegoles Local (free, runs on this Mac).",
    });
    // An integrity failure is the local model's, even though it mentions checksums.
    const integrity = humanizeError("The local model failed its integrity check (sha256 mismatch for model.safetensors). Remove it and set it up again in Settings.", "run");
    expect(integrity).toMatchObject({ scope: "run", title: "Pegoles Local needs to be set up again.", retryable: false });
    expect(integrity.detail).toMatch(/sha256 mismatch/);
  });

  it("names the local model's failures calmly, wherever they appear", () => {
    expect(humanizeError("The model could not continue: model unavailable: local model runtime stopped unexpectedly: exit 9", "run").title).toBe("Pegoles Local stopped unexpectedly.");
    expect(humanizeError("The model could not continue: model unavailable: not enough memory to run the local model: 3 GB", "run").title).toBe("Not enough free memory for Pegoles Local.");
    expect(humanizeError("local model runtime is not installed: the Pegoles Local runtime is not set up on this Mac", "run").title).toBe("Pegoles Local can’t run on this Mac yet.");
    expect(stopReason("Stopped: the local model did not produce a valid action after 3 attempts.")).toBe("Stopped: Pegoles Local couldn’t decide on a next step.");
    expect(stopReason("Stopped: the local model kept repeating the same action without any visible change.")).toBe("Stopped: it kept repeating the same action.");
    // Only planner failures are rewritten; a person's stop stays in Pegoles' words.
    expect(stopReason("Stopped by the user.")).toBe("Stopped by the user.");
    expect(stopReason("task 01 is already running")).toBe("task 01 is already running");
  });

  it("keeps a failed start with its task, in words people can act on", () => {
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
    // Windows before its image is published: say so, don't suggest a retry.
    const unpublished = humanizeError("computer image missing: pegoles-base-x64-0.1 has no release download in this build", "computer");
    expect(unpublished).toMatchObject({ retryable: false });
    expect(unpublished.title).toMatch(/isn’t available for this (Mac|PC) yet/);
    expect(unpublished.hint).toMatch(/hasn’t been published yet/);
  });

  it("says Windows and guest failures as sentences people can act on, detail kept", () => {
    const hr = humanizeError("computer error: backend error: HcsCreateComputeSystem failed: HRESULT 0x80370102 (call rejected)", "computer");
    expect(hr).toMatchObject({ title: "Virtualization needs to be turned on.", retryable: false });
    expect(hr.hint).toMatch(/its own isolated computer/);
    expect(hr.detail).toContain("0x80370102");
    expect(humanizeError("guest handshake timeout after 10000ms", "computer").title).toBe("Pegoles couldn’t start its computer.");
    expect(humanizeError("guest handshake timeout after 10000ms", "computer").hint).toMatch(/Technical details/);
    expect(humanizeError("model worker OOM", "computer").title).toBe("Pegoles ran out of available memory.");
    expect(humanizeError("cannot reach pegoles-vm-broker: pipe not found", "computer").hint).toMatch(/Reinstalling Pegoles/);
    // The local model's own memory message keeps its more precise words.
    expect(humanizeError("not enough memory to run the local model: 3 GB", "run").title).toBe("Not enough free memory for Pegoles Local.");
  });

  it("never loses the detail", () => {
    expect(humanizeError("", "general").detail).toBe("No details were reported.");
    expect(errorText({ code: 7 })).toBe('{"code":7}');
  });
});

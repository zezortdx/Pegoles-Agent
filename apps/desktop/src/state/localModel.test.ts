import { describe, expect, it } from "vitest";
import { canSetUp, chosenModel, localView, percentOf, progressText, sentence, setupAction } from "./localModel";
import { installOf, intelligenceOf, MAI, QWEN } from "./intelligenceFixture";
import { formatBytes, formatMemory, quantizationLabel } from "../lib/format";

describe("localView: Pegoles Local from what Core reports", () => {
  it("waits for Core before saying anything", () => {
    expect(localView(null)).toMatchObject({ stage: "checking", model: null });
  });

  it("is not set up until the store has the model", () => {
    const view = localView(intelligenceOf());
    expect(view).toMatchObject({ stage: "not-set-up", model: { id: MAI.id }, problem: null });
    expect(canSetUp(view)).toBe(true);
    expect(setupAction(view)).toBe("Set up");
  });

  it("follows a setup job for the chosen model, and only that one", () => {
    const downloading = localView(intelligenceOf({ local: { install: installOf("downloading", 935_000_000) } }));
    expect(downloading).toMatchObject({ stage: "preparing", step: "downloading", doneBytes: 935_000_000, totalBytes: MAI.size_bytes });
    expect(canSetUp(downloading)).toBe(false);
    const other = localView(intelligenceOf({ local: { install: installOf("downloading", 5, { model: QWEN.id }) } }));
    expect(other.stage).toBe("not-set-up");
  });

  it("reads per-file checks as downloading, and only says verifying once every byte is in", () => {
    expect(localView(intelligenceOf({ local: { install: installOf("verifying", 400_000_000) } })).step).toBe("downloading");
    expect(localView(intelligenceOf({ local: { install: installOf("verifying", MAI.size_bytes) } })).step).toBe("verifying");
    expect(localView(intelligenceOf({ local: { install: installOf("finalizing", MAI.size_bytes) } })).step).toBe("finishing");
  });

  it("is ready once installed with a runtime, and running while the worker has it loaded", () => {
    expect(localView(intelligenceOf({ model: { state: "installed" } }))).toMatchObject({ stage: "ready", running: false, footprintBytes: null });
    // A setup that just finished counts before Core is read again.
    expect(localView(intelligenceOf({ local: { install: installOf("ready", MAI.size_bytes) } })).stage).toBe("ready");
    const running = localView(intelligenceOf({ model: { state: "installed" }, local: { loaded_model: MAI.id, worker_footprint_bytes: 2_463_000_000 } }));
    expect(running).toMatchObject({ stage: "ready", running: true, footprintBytes: 2_463_000_000 });
  });

  it("says a failed setup in Core's words, and how it can resume", () => {
    const failed = localView(intelligenceOf({
      model: { state: "partial", partial_bytes: 800_000_000 },
      local: { install: installOf("failed", 0, { error: "The download was interrupted. Check your connection and retry; it resumes where it stopped.", error_kind: "network" }) },
    }));
    expect(failed).toMatchObject({ stage: "failed", errorKind: "network", doneBytes: 800_000_000 });
    expect(failed.error).toMatch(/^The download was interrupted/);
    expect(setupAction(failed)).toBe("Resume");
    const corrupted = localView(intelligenceOf({ local: { install: installOf("failed", 0, { error: "…discarded.", error_kind: "corrupted" }) } }));
    expect(setupAction(corrupted)).toBe("Download again");
    const disk = localView(intelligenceOf({ local: { install: installOf("failed", 0, { error: "Not enough disk space", error_kind: "disk_space" }) } }));
    expect(setupAction(disk)).toBe("Try again");
  });

  it("pauses at the bytes on disk after a cancel, and asks to set up again when files are damaged", () => {
    const paused = localView(intelligenceOf({ model: { state: "partial", partial_bytes: 823_000_000 }, local: { install: installOf("cancelled", 0) } }));
    expect(paused).toMatchObject({ stage: "paused", doneBytes: 823_000_000, totalBytes: MAI.size_bytes });
    expect(setupAction(paused)).toBe("Resume");
    // A cancel before any byte arrived leaves nothing to resume.
    expect(localView(intelligenceOf({ model: { state: "partial", partial_bytes: 0 } })).stage).toBe("not-set-up");
    const damaged = localView(intelligenceOf({ model: { state: "invalid", invalid_reason: "config.json is missing" } }));
    expect(damaged).toMatchObject({ stage: "damaged", reason: "config.json is missing" });
    expect(setupAction(damaged)).toBe("Set up again");
  });

  it("says plainly when this Mac can't run it", () => {
    const intel = intelligenceOf({ local: { apple_silicon: false, runtime_ready: false, runtime_problem: "Pegoles Local needs a Mac with Apple silicon." } });
    expect(localView(intel)).toMatchObject({ stage: "unsupported", problem: "Pegoles Local needs a Mac with Apple silicon." });
    expect(canSetUp(localView(intel))).toBe(false);
    const missing = intelligenceOf({
      model: { state: "installed" },
      local: { runtime_ready: false, runtime_problem: "local model runtime is not installed: the Pegoles Local runtime is not set up on this Mac" },
    });
    expect(localView(missing)).toMatchObject({ stage: "downloaded", problem: "The Pegoles Local runtime is not set up on this Mac." });
  });

  it("uses the chosen model, else the catalog default", () => {
    expect(chosenModel(intelligenceOf({ models: [MAI, QWEN], local_model: QWEN.id }))?.id).toBe(QWEN.id);
    expect(chosenModel(intelligenceOf({ models: [MAI, QWEN], local_model: "gone" }))?.id).toBe(MAI.id);
  });
});

describe("numbers people read", () => {
  it("counts downloads like Finder and memory like macOS", () => {
    expect(formatBytes(MAI.size_bytes)).toBe("2.2 GB");
    expect(formatBytes(935_000_000)).toBe("935 MB");
    expect(formatBytes(999_700_000)).toBe("1.0 GB");
    expect(progressText(935_000_000, MAI.size_bytes)).toBe("935 MB of 2.2 GB");
    expect(formatMemory(18 * 2 ** 30)).toBe("18 GB");
    expect(formatMemory(2_463_000_000)).toBe("2.3 GB");
    expect(quantizationLabel("6bit")).toBe("6-bit");
    expect(quantizationLabel("q4_k_m")).toBe("q4_k_m");
  });

  it("only gives a percentage for a real measure", () => {
    expect(percentOf(935_000_000, MAI.size_bytes)).toBe(41);
    expect(percentOf(5, 0)).toBeNull();
    expect(percentOf(undefined, 10)).toBeNull();
    expect(sentence("the download was interrupted")).toBe("The download was interrupted.");
  });
});

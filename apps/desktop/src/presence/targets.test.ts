import { describe, expect, it } from "vitest";
import { PRESENCE_LABEL, PRESENCE_MODES } from "./modes";
import { poseVariables, presenceTargets } from "./targets";

describe("presence targets", () => {
  it("defines targets and an accessible sentence for every mode", () => {
    for (const mode of PRESENCE_MODES) {
      const t = presenceTargets(mode);
      expect(Number.isFinite(t.breathAmp) && Number.isFinite(t.eyeScaleY) && Number.isFinite(t.halo)).toBe(true);
      expect(PRESENCE_LABEL[mode]).toMatch(/Pegoles/);
    }
  });

  it("uses the amber attention tint only when Pegoles needs the person", () => {
    for (const mode of PRESENCE_MODES) {
      const { warn } = presenceTargets(mode);
      if (mode === "needs-user") expect(warn).toBeGreaterThan(0.5);
      else expect(warn).toBe(0);
    }
  });

  it("keeps breathing within 1.2% and never animates the thought field at rest", () => {
    for (const mode of PRESENCE_MODES) expect(presenceTargets(mode).breathAmp).toBeLessThanOrEqual(0.012);
    for (const mode of ["idle", "offline", "blocked", "done", "error"] as const) {
      expect(presenceTargets(mode).field).toBe(0);
      expect(presenceTargets(mode).filament).toBe(0);
    }
  });

  it("removes spatial motion under reduced motion but keeps the state legible", () => {
    for (const mode of PRESENCE_MODES) {
      const full = presenceTargets(mode);
      const still = presenceTargets(mode, { reducedMotion: true });
      expect(still).toMatchObject({ breathAmp: 0, flow: 0, gaze: 0, life: false });
      expect(still.warn).toBe(full.warn);
      expect(still.lid).toBe(full.lid);
      expect(still.eyeGlow).toBe(full.eyeGlow);
    }
  });

  it("has idle life only while resting, attentive or waiting", () => {
    const lively = PRESENCE_MODES.filter((mode) => presenceTargets(mode).life);
    expect(lively).toEqual(["idle", "attentive", "waiting"]);
  });

  it("makes working states legible at a glance, but thin and cold", () => {
    for (const mode of ["thinking", "planning", "working", "using-computer"] as const) {
      const t = presenceTargets(mode);
      expect(t.field).toBeGreaterThanOrEqual(0.6);
      expect(t.filament).toBeGreaterThanOrEqual(0.85);
      expect(t.warn).toBe(0);
    }
  });

  it("exposes the same pose to CSS", () => {
    const vars = poseVariables(presenceTargets("offline"));
    expect(vars["--p-lid"]).toBe("0.72");
    expect(vars["--p-dim"]).toBe(String(1 - 0.55));
    expect(poseVariables(presenceTargets("working"))["--p-flow-period"]).toBe("4.20s");
  });
});

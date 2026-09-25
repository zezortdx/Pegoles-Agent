import { describe, expect, it } from "vitest";
import { ambientPaused, frameDelay, frameTooSoon, QualityGovernor, SVG_RUNG, UNFOCUSED_WORK_FPS, type FrameInput } from "./governor";
import { wantsGl } from "./quality";

const base: FrameInput = { visible: true, paused: false, unsettled: false, pulsesActive: false, fpsCap: 24, reducedMotion: false, focused: true };

describe("frame governor", () => {
  it("renders every frame while anything moves", () => {
    expect(frameDelay({ ...base, unsettled: true })).toBe(0);
    expect(frameDelay({ ...base, pulsesActive: true })).toBe(0);
  });

  it("stops entirely when hidden or paused, even mid-transition", () => {
    expect(frameDelay({ ...base, visible: false, unsettled: true })).toBeNull();
    expect(frameDelay({ ...base, paused: true, unsettled: true })).toBeNull();
  });

  it("throttles settled life to the mode cap, lower when unfocused", () => {
    expect(frameDelay(base)).toBeCloseTo(1000 / 24);
    expect(frameDelay({ ...base, focused: false })).toBeCloseTo(1000 / UNFOCUSED_WORK_FPS);
  });

  it("renders only on change for still poses and reduced motion", () => {
    expect(frameDelay({ ...base, fpsCap: 0 })).toBeNull();
    expect(frameDelay({ ...base, reducedMotion: true })).toBeNull();
  });

  it("pauses decorative life with the ambient gate but keeps real work alive when merely unfocused", () => {
    const blurred = { running: false, visible: true, focused: false, externallyHidden: false };
    expect(ambientPaused(blurred, false)).toBe(true);
    expect(ambientPaused(blurred, true)).toBe(false);
    expect(ambientPaused({ ...blurred, visible: false }, true)).toBe(true);
    expect(ambientPaused({ ...blurred, externallyHidden: true }, true)).toBe(true);
    expect(ambientPaused({ ...blurred, focused: true }, true)).toBe(true);
    expect(ambientPaused(null, false)).toBe(false);
  });
});

describe("60 fps clamp", () => {
  it("skips every other frame on a 120 Hz display but never drops a 60 Hz frame", () => {
    expect(frameTooSoon(1000 + 8.33, 1000)).toBe(true);
    expect(frameTooSoon(1000 + 16.67, 1000)).toBe(false);
    expect(frameTooSoon(1000 + 15.4, 1000)).toBe(false); // rAF jitter at 60 Hz
    expect(frameTooSoon(5, 0)).toBe(false); // first frame
  });
});

describe("auto quality", () => {
  it("steps down after sustained slow frames and hands over to SVG last", () => {
    const governor = new QualityGovernor();
    let now = 0;
    const run = (interval: number, ms: number) => {
      for (let t = 0; t < ms; t += interval) { now += interval; governor.sample(interval, now); }
    };
    run(40, 2100);
    expect(governor.rung).toBe(1);
    run(40, 20_000);
    expect(governor.rung).toBe(SVG_RUNG);
  });

  it("steps back up only after a long run of good frames", () => {
    const governor = new QualityGovernor();
    let now = 0;
    const run = (interval: number, ms: number) => {
      for (let t = 0; t < ms; t += interval) { now += interval; governor.sample(interval, now); }
    };
    run(40, 2100);
    expect(governor.rung).toBe(1);
    run(16, 9000);
    expect(governor.rung).toBe(1);
    run(16, 2000);
    expect(governor.rung).toBe(0);
  });

  it("ignores gaps from paused loops", () => {
    const governor = new QualityGovernor();
    for (let i = 0; i < 100; i += 1) governor.sample(1000, i * 1000);
    expect(governor.rung).toBe(0);
  });
});

describe("renderer choice", () => {
  const ok = { quality: "auto" as const, tier: "full" as const, size: 152, webgl2: true, disabled: false };
  it("uses WebGL for large presences when the tier allows", () => {
    expect(wantsGl(ok)).toBe(true);
    expect(wantsGl({ ...ok, tier: "reduced" })).toBe(false);
    expect(wantsGl({ ...ok, tier: "reduced", quality: "full" })).toBe(true);
  });
  it("falls back to SVG for Reduced, minimal tier, small sizes, no WebGL2 or a disabled session", () => {
    expect(wantsGl({ ...ok, quality: "reduced" })).toBe(false);
    expect(wantsGl({ ...ok, quality: "full", tier: "minimal" })).toBe(false);
    expect(wantsGl({ ...ok, size: 40 })).toBe(false);
    expect(wantsGl({ ...ok, webgl2: false })).toBe(false);
    expect(wantsGl({ ...ok, disabled: true })).toBe(false);
  });
});

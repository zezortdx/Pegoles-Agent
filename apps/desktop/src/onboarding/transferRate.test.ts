import { describe, expect, it } from "vitest";
import { addSample, EMPTY_RATE, estimate, formatRate, formatTimeLeft } from "./transferRate";

describe("download speed and time left", () => {
  it("says nothing until a few seconds of steady samples", () => {
    let s = addSample(EMPTY_RATE, { bytes: 0, at: 0 });
    s = addSample(s, { bytes: 10e6, at: 1000 });
    expect(estimate(s, 100e6, 1000)).toBeNull();
    for (let t = 2; t <= 4; t += 1) s = addSample(s, { bytes: t * 10e6, at: t * 1000 });
    const e = estimate(s, 100e6, 4000);
    expect(e?.bytesPerSecond).toBeCloseTo(10e6, -5);
    expect(e?.secondsLeft).toBe(6);
  });

  it("a stall or a restart hides the estimate", () => {
    let s = EMPTY_RATE;
    for (let t = 0; t <= 5; t += 1) s = addSample(s, { bytes: t * 1e6, at: t * 1000 });
    expect(estimate(s, 50e6, 5000)).not.toBeNull();
    expect(estimate(s, 50e6, 20_000)).toBeNull();
    s = addSample(s, { bytes: 0, at: 21_000 });
    expect(estimate(s, 50e6, 21_000)).toBeNull();
  });

  it("reads calmly", () => {
    expect(formatRate(8.44e6)).toBe("8.4 MB/s");
    expect(formatRate(25e6)).toBe("25 MB/s");
    expect(formatRate(40e3)).toBe("40 KB/s");
    expect(formatTimeLeft(30)).toBe("Less than a minute left");
    expect(formatTimeLeft(185)).toBe("About 3 minutes left");
    expect(formatTimeLeft(3700)).toBe("About 1 hour left");
  });
});

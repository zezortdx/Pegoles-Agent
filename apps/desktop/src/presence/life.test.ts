import { describe, expect, it } from "vitest";
import {
  BLINK_MAX_MS, BLINK_MIN_MS, createThrottle, DOUBLE_BLINK_GAP_MS, GLANCE_MAX, GLANCE_MAX_MS, GLANCE_MIN, GLANCE_MIN_MS,
  LifeScheduler, planBlink, planGlance, type Clock, type Offset,
} from "./life";

/** A deterministic clock: timers fire only when time is advanced. */
function fakeClock() {
  let now = 0;
  let next = 1;
  const pending = new Map<number, { at: number; fn: () => void }>();
  const clock: Clock = {
    now: () => now,
    setTimeout: (fn, ms) => { const id = next++; pending.set(id, { at: now + ms, fn }); return id; },
    clearTimeout: (id) => { pending.delete(id); },
  };
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      const due = [...pending.entries()].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      pending.delete(due[0]);
      now = due[1].at;
      due[1].fn();
    }
    now = end;
  };
  return { clock, advance, pending };
}

/** Cycles through fixed values in [0, 1). */
const sequence = (...values: number[]) => { let i = 0; return () => values[i++ % values.length]; };

describe("idle life plans", () => {
  it("blinks every 2.8–6.5 s, with a double blink about one time in five", () => {
    expect(planBlink(sequence(0, 0.5)).delayMs).toBe(BLINK_MIN_MS);
    expect(planBlink(sequence(0.9999999, 0.5)).delayMs).toBeCloseTo(BLINK_MAX_MS, 0);
    expect(planBlink(sequence(0.3, 0.1)).double).toBe(true);
    expect(planBlink(sequence(0.3, 0.2)).double).toBe(false);
    let doubles = 0;
    let seed = 7;
    const lcg = () => { seed = (seed * 1664525 + 1013904223) % 4294967296; return seed / 4294967296; };
    for (let i = 0; i < 4000; i += 1) if (planBlink(lcg).double) doubles += 1;
    expect(doubles / 4000).toBeGreaterThan(0.17);
    expect(doubles / 4000).toBeLessThan(0.23);
  });

  it("glances 4–7% of the mark every 7–13 s, travelling less vertically", () => {
    for (const r of [0, 0.25, 0.5, 0.99]) {
      const plan = planGlance(sequence(r, r, r, r));
      expect(plan.delayMs).toBeGreaterThanOrEqual(GLANCE_MIN_MS);
      expect(plan.delayMs).toBeLessThanOrEqual(GLANCE_MAX_MS);
      expect(Math.abs(plan.offset.x)).toBeLessThanOrEqual(GLANCE_MAX);
      expect(Math.abs(plan.offset.y)).toBeLessThanOrEqual(GLANCE_MAX * 0.6);
    }
    const horizontal = planGlance(sequence(0, 0, 0, 0));
    expect(horizontal.offset.x).toBeCloseTo(GLANCE_MIN);
  });
});

describe("life scheduler", () => {
  it("blinks on its schedule, doubles when planned, and stops cleanly", () => {
    const { clock, advance, pending } = fakeClock();
    const blinks: number[] = [];
    // blink: delay 0 → 2800 ms, double (0.1 < 0.2); glance far away (0.99 → ~13 s).
    const life = new LifeScheduler({ blink: () => blinks.push(clock.now()), glance: () => undefined }, clock, sequence(0, 0.1, 0.99, 0.99, 0.99, 0.99));
    life.start();
    advance(BLINK_MIN_MS - 1);
    expect(blinks).toEqual([]);
    advance(1);
    expect(blinks).toEqual([BLINK_MIN_MS]);
    advance(DOUBLE_BLINK_GAP_MS);
    expect(blinks).toEqual([BLINK_MIN_MS, BLINK_MIN_MS + DOUBLE_BLINK_GAP_MS]);
    life.stop();
    expect(pending.size).toBe(0);
    advance(60_000);
    expect(blinks).toHaveLength(2);
  });

  it("glances, holds, returns to rest, and returns to rest when stopped mid-glance", () => {
    const { clock, advance } = fakeClock();
    const glances: (Offset | null)[] = [];
    // First draw is for the blink (put it far away), then the glance plan.
    const life = new LifeScheduler({ blink: () => undefined, glance: (o) => glances.push(o) }, clock, sequence(0.99, 0.5, 0, 0.5, 0, 0));
    life.start();
    advance(GLANCE_MIN_MS);
    expect(glances).toHaveLength(1);
    expect(glances[0]).not.toBeNull();
    advance(800);
    expect(glances[1]).toBeNull();
    advance(GLANCE_MIN_MS);
    expect(glances).toHaveLength(3);
    life.stop();
    expect(glances[3]).toBeNull();
    expect(life.active).toBe(false);
  });

  it("start is idempotent", () => {
    const { clock, pending } = fakeClock();
    const life = new LifeScheduler({ blink: () => undefined, glance: () => undefined }, clock, sequence(0.5));
    life.start();
    life.start();
    expect(pending.size).toBe(2);
  });
});

describe("keystroke throttle", () => {
  it("lets at most one nudge through per 90 ms", () => {
    let now = 0;
    const gate = createThrottle(90, () => now);
    const passed: number[] = [];
    for (now = 0; now <= 300; now += 20) if (gate()) passed.push(now);
    expect(passed).toEqual([0, 100, 200, 300]);
  });
});

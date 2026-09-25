/**
 * Idle life: randomized blinks and glances, and small timing helpers. All
 * timing goes through an injected clock so it is tested without real time;
 * nothing here touches React state.
 */

export type Random = () => number;

export interface Clock {
  now(): number;
  setTimeout(callback: () => void, ms: number): number;
  clearTimeout(id: number): void;
}

export const systemClock: Clock = {
  now: () => (typeof performance !== "undefined" ? performance.now() : Date.now()),
  setTimeout: (callback, ms) => window.setTimeout(callback, ms),
  clearTimeout: (id) => window.clearTimeout(id),
};

export const BLINK_MIN_MS = 2800;
export const BLINK_MAX_MS = 6500;
export const DOUBLE_BLINK_CHANCE = 0.2;
/** Gap between the two blinks of a double blink. */
export const DOUBLE_BLINK_GAP_MS = 260;
export const GLANCE_MIN_MS = 7000;
export const GLANCE_MAX_MS = 13_000;
/** Eye travel of a glance, as a fraction of the mark size. */
export const GLANCE_MIN = 0.04;
export const GLANCE_MAX = 0.07;
export const GLANCE_HOLD_MIN_MS = 800;
export const GLANCE_HOLD_MAX_MS = 1400;

const between = (random: Random, min: number, max: number) => min + (max - min) * random();

export interface BlinkPlan {
  readonly delayMs: number;
  readonly double: boolean;
}

export function planBlink(random: Random): BlinkPlan {
  return { delayMs: between(random, BLINK_MIN_MS, BLINK_MAX_MS), double: random() < DOUBLE_BLINK_CHANCE };
}

export interface Offset {
  readonly x: number;
  readonly y: number;
}

export interface GlancePlan {
  readonly delayMs: number;
  readonly holdMs: number;
  /** Eye offset as a fraction of the mark size (y down). */
  readonly offset: Offset;
}

export function planGlance(random: Random): GlancePlan {
  const angle = random() * Math.PI * 2;
  const distance = between(random, GLANCE_MIN, GLANCE_MAX);
  return {
    delayMs: between(random, GLANCE_MIN_MS, GLANCE_MAX_MS),
    holdMs: between(random, GLANCE_HOLD_MIN_MS, GLANCE_HOLD_MAX_MS),
    // The face is wider than it is tall: vertical glances travel less.
    offset: { x: Math.cos(angle) * distance, y: Math.sin(angle) * distance * 0.6 },
  };
}

export interface LifeCallbacks {
  blink(): void;
  /** An offset to look toward, or null to return to rest. */
  glance(offset: Offset | null): void;
}

/**
 * Runs blinks and glances on independent random schedules. `start` is
 * idempotent; `stop` cancels everything and returns the eyes to rest.
 */
export class LifeScheduler {
  private readonly timers = new Set<number>();
  private running = false;
  private glancing = false;

  constructor(
    private readonly callbacks: LifeCallbacks,
    private readonly clock: Clock = systemClock,
    private readonly random: Random = Math.random,
  ) {}

  get active(): boolean {
    return this.running;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.scheduleBlink();
    this.scheduleGlance();
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    for (const id of this.timers) this.clock.clearTimeout(id);
    this.timers.clear();
    if (this.glancing) {
      this.glancing = false;
      this.callbacks.glance(null);
    }
  }

  private after(ms: number, callback: () => void): void {
    const id = this.clock.setTimeout(() => {
      this.timers.delete(id);
      if (this.running) callback();
    }, ms);
    this.timers.add(id);
  }

  private scheduleBlink(): void {
    const plan = planBlink(this.random);
    this.after(plan.delayMs, () => {
      this.callbacks.blink();
      if (plan.double) this.after(DOUBLE_BLINK_GAP_MS, () => this.callbacks.blink());
      this.scheduleBlink();
    });
  }

  private scheduleGlance(): void {
    const plan = planGlance(this.random);
    this.after(plan.delayMs, () => {
      this.glancing = true;
      this.callbacks.glance(plan.offset);
      this.after(plan.holdMs, () => {
        this.glancing = false;
        this.callbacks.glance(null);
        this.scheduleGlance();
      });
    });
  }
}

/** A gate that opens at most once per interval (keystroke reactions). */
export function createThrottle(intervalMs: number, now: () => number = systemClock.now): () => boolean {
  let last = -Infinity;
  return () => {
    const t = now();
    if (t - last < intervalMs) return false;
    last = t;
    return true;
  };
}

/** Timeouts owned by one presence, cleared together on unmount. */
export class Timers {
  private readonly ids = new Set<number>();

  constructor(private readonly clock: Clock = systemClock) {}

  after(ms: number, callback: () => void): void {
    const id = this.clock.setTimeout(() => {
      this.ids.delete(id);
      callback();
    }, ms);
    this.ids.add(id);
  }

  clear(): void {
    for (const id of this.ids) this.clock.clearTimeout(id);
    this.ids.clear();
  }
}

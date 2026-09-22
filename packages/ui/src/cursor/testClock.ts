/** Deterministic frame clock for AgentCursor tests (not exported from the package). */
import type { CursorTimers, FrameScheduler } from "./AgentCursorController.js";

export interface FakeClock extends FrameScheduler {
  /** Advance time and run the callbacks queued for the next frame. */
  frame(ms?: number): void;
  /** Run frames until no callback is queued (or `max` frames). Returns frames run. */
  runUntilIdle(ms?: number, max?: number): number;
  readonly pending: number;
  readonly time: number;
  readonly timers: CursorTimers & { advance(ms: number): void };
}

export function createFakeClock(): FakeClock {
  let t = 1000;
  let nextId = 1;
  const queue = new Map<number, (now: number) => void>();
  let timeouts: { at: number; fn: () => void; id: number }[] = [];

  const clock: FakeClock = {
    request(cb) {
      const id = nextId++;
      queue.set(id, cb);
      return id;
    },
    cancel(id) {
      queue.delete(id);
    },
    now: () => t,
    frame(ms = 1000 / 60) {
      t += ms;
      const cbs = [...queue.values()];
      queue.clear();
      for (const cb of cbs) cb(t);
    },
    runUntilIdle(ms = 1000 / 60, max = 10_000) {
      let n = 0;
      while (queue.size > 0 && n < max) {
        clock.frame(ms);
        n += 1;
      }
      return n;
    },
    get pending() {
      return queue.size;
    },
    get time() {
      return t;
    },
    timers: {
      setTimeout(fn, ms) {
        const id = nextId++;
        timeouts.push({ at: t + ms, fn, id });
        return id;
      },
      clearTimeout(handle) {
        timeouts = timeouts.filter((x) => x.id !== handle);
      },
      advance(ms) {
        t += ms;
        const due = timeouts.filter((x) => x.at <= t);
        timeouts = timeouts.filter((x) => x.at > t);
        for (const x of due) x.fn();
      },
    },
  };
  return clock;
}

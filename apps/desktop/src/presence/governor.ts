/**
 * Pure frame-loop and quality policy for the WebGL presence. Kept free of
 * DOM and GL so it is unit tested; the renderer only asks these questions.
 */

export interface FrameInput {
  /** Window visible and not minimized. */
  readonly visible: boolean;
  /** Off screen, or the ambient gate paused decorative life. */
  readonly paused: boolean;
  /** Springs still travelling or a one-shot gesture in flight. */
  readonly unsettled: boolean;
  /** A thought-field ripple is still travelling. */
  readonly pulsesActive: boolean;
  /** Mode frame cap once settled; 0 = render on change only. */
  readonly fpsCap: number;
  readonly reducedMotion: boolean;
  readonly focused: boolean;
}

/** Frames per second while real work continues in an unfocused window. */
export const UNFOCUSED_WORK_FPS = 8;

/**
 * Milliseconds until the next frame: 0 = next animation frame, null = stop
 * (the last frame stays on screen until something changes).
 */
export function frameDelay(input: FrameInput): number | null {
  if (!input.visible || input.paused) return null;
  if (input.unsettled || input.pulsesActive) return 0;
  if (input.reducedMotion || input.fpsCap <= 0) return null;
  const cap = input.focused ? input.fpsCap : Math.min(input.fpsCap, UNFOCUSED_WORK_FPS);
  return 1000 / cap;
}

/** Minimum spacing between drawn frames: never faster than 60 fps. */
export const MIN_FRAME_MS = 1000 / 60;
/** rAF jitter allowance, so a 60 Hz display never drops to 30 fps. */
const FRAME_SLACK_MS = 2.5;

/**
 * True when an animation frame arrives too soon after the last drawn one
 * (120 Hz displays): skip it and wait for the next. 60 Hz frames always pass.
 */
export function frameTooSoon(now: number, lastDraw: number): boolean {
  return lastDraw > 0 && now - lastDraw < MIN_FRAME_MS - FRAME_SLACK_MS;
}

export interface AmbientLike {
  readonly running: boolean;
  readonly visible: boolean;
  readonly focused: boolean;
  readonly externallyHidden: boolean;
}

/**
 * Decorative life stops whenever the ambient gate says so (blurred, idle,
 * hidden). Real work keeps a low frame rate while merely unfocused so a
 * glance never shows a frozen "working" Pegoles.
 */
export function ambientPaused(snapshot: AmbientLike | null, workMode: boolean): boolean {
  if (!snapshot) return false;
  if (!snapshot.visible || snapshot.externallyHidden) return true;
  if (snapshot.running) return false;
  return !(workMode && !snapshot.focused);
}

/** Quality ladder, best first. The last rung hands over to the SVG renderer. */
export const QUALITY_LADDER = [
  { dpr: 2, detail: 1 },
  { dpr: 1.5, detail: 1 },
  { dpr: 1.25, detail: 1 },
  { dpr: 1.25, detail: 0 },
] as const;
export const SVG_RUNG = QUALITY_LADDER.length;

const EMA_ALPHA = 0.1;
const SLOW_FACTOR = 1.5;
const GOOD_FACTOR = 1.1;
const STEP_DOWN_AFTER_MS = 2000;
const STEP_UP_AFTER_MS = 10_000;

/**
 * Auto quality: an EMA of animation-frame intervals, sampled only while the
 * loop runs every frame. Sustained slow frames step down one rung; a long
 * run of good frames steps back up (hysteresis).
 */
export class QualityGovernor {
  rung = 0;
  private ema = 0;
  private slowSince: number | null = null;
  private goodSince: number | null = null;

  constructor(private readonly budgetMs = 1000 / 60) {}

  /** Returns true when the rung changed. */
  sample(intervalMs: number, now: number): boolean {
    if (!(intervalMs > 0) || intervalMs > 250) return false; // tab switches, debugger pauses
    this.ema = this.ema === 0 ? intervalMs : this.ema + EMA_ALPHA * (intervalMs - this.ema);
    if (this.ema > this.budgetMs * SLOW_FACTOR) {
      this.goodSince = null;
      this.slowSince ??= now;
      if (now - this.slowSince >= STEP_DOWN_AFTER_MS && this.rung < SVG_RUNG) {
        this.rung += 1;
        this.slowSince = null;
        this.ema = 0;
        return true;
      }
    } else if (this.ema <= this.budgetMs * GOOD_FACTOR) {
      this.slowSince = null;
      this.goodSince ??= now;
      if (now - this.goodSince >= STEP_UP_AFTER_MS && this.rung > 0 && this.rung < SVG_RUNG) {
        this.rung -= 1;
        this.goodSince = null;
        return true;
      }
    } else {
      this.slowSince = null;
      this.goodSince = null;
    }
    return false;
  }
}

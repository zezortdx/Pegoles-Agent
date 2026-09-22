/**
 * AgentCursorController — imperative, React-free driver of the AgentCursor.
 *
 * Performance architecture:
 * - Positions never touch React state or context. Every frame writes
 *   `transform` / `opacity` straight onto a handful of DOM nodes inside an
 *   isolated overlay layer.
 * - The rAF loop runs ONLY while something moves (spring not settled, or
 *   the trail still catching up) and stops the frame it settles. It never
 *   runs while the document is hidden: hiding snaps to the target.
 * - Click ripple / press feedback / reduced-motion fades use WAAPI
 *   (compositor), not the loop.
 */
import { effectsTiers, type EffectsTier } from "../tokens/effects.js";
import { easing } from "../tokens/motion.js";
import { canTransition, transitionCursor, type CursorEvent, type CursorState } from "./cursorMachine.js";
import { AGENT_CURSOR_SPRING, isSettled, stepSpring, type SpringAxis, type SpringConfig } from "./spring.js";

export interface FrameScheduler {
  request(callback: (now: number) => void): number;
  cancel(handle: number): void;
  now(): number;
}

export interface CursorTimers {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface AgentCursorElements {
  /** Overlay root; receives data-state / data-tier / data-rm. */
  readonly root: HTMLElement;
  /** Positioned so its origin is the pointer's hotspot. */
  readonly pointer: HTMLElement;
  /** Inner node for press feedback (scale), so it never fights the position. */
  readonly pointerBody: HTMLElement;
  /** Up to MAX_TRAIL_ELEMENTS nodes; extras are hidden per tier. */
  readonly trail: readonly HTMLElement[];
  readonly ripple: HTMLElement;
  /** 1 px-wide base line, origin at its left-middle. */
  readonly dragPath: HTMLElement;
}

export interface AgentCursorControllerOptions {
  readonly elements: AgentCursorElements;
  readonly tier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly spring?: SpringConfig;
  readonly scheduler?: FrameScheduler;
  readonly timers?: CursorTimers;
  readonly document?: Document;
}

export interface CursorPoint {
  readonly x: number;
  readonly y: number;
}

/** Rendered trail nodes (Full). */
export const MAX_TRAIL_ELEMENTS = 4;
/** Trail samples per tier: a few in Full, one in Reduced, none in Minimal. */
export const TRAIL_BY_TIER: Readonly<Record<EffectsTier, number>> = { full: 4, reduced: 1, minimal: 0 };
/** Each trail sample lags the previous one by this much time. */
export const TRAIL_LAG_MS = 18;
/** Click ripple / refraction duration (150–180 ms). */
export const CLICK_MS = 170;
/** Reduced motion: moves become a quick fade at the destination. */
export const RM_FADE_MS = 160;
const DRAG_FADE_DELAY_MS = 120;
/** Speed (px/s) at which the trail reaches full opacity. */
const TRAIL_FULL_SPEED = 900;
const MAX_DT_S = 0.1;
const HISTORY = 48;

interface Sample {
  readonly t: number;
  readonly x: number;
  readonly y: number;
}

const rafScheduler: FrameScheduler = {
  request: (cb) => requestAnimationFrame(cb),
  cancel: (h) => cancelAnimationFrame(h),
  now: () => performance.now(),
};

const defaultTimers: CursorTimers = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (h) => globalThis.clearTimeout(h as ReturnType<typeof setTimeout>),
};

function finite(n: number): number {
  return Number.isFinite(n) ? n : 0;
}

function play(el: HTMLElement, frames: Keyframe[], ms: number): void {
  if (typeof el.animate === "function") {
    el.animate(frames, { duration: ms, easing: easing.out, fill: "none" });
  }
}

export function trailLengthFor(tier: EffectsTier, reducedMotion: boolean): number {
  if (reducedMotion) return 0;
  return Math.min(TRAIL_BY_TIER[tier], effectsTiers[tier].cursorTrailLength, MAX_TRAIL_ELEMENTS);
}

export class AgentCursorController {
  private readonly el: AgentCursorElements;
  private readonly scheduler: FrameScheduler;
  private readonly timers: CursorTimers;
  private readonly doc: Document | null;
  private readonly springConfig: SpringConfig;
  private tier: EffectsTier;
  private reducedMotion: boolean;
  private current: CursorState = "hidden";
  private x: SpringAxis = { position: 0, velocity: 0 };
  private y: SpringAxis = { position: 0, velocity: 0 };
  private target: CursorPoint = { x: 0, y: 0 };
  private dragFrom: CursorPoint | null = null;
  private history: Sample[] = [];
  private lastMotion = -Infinity;
  private lastTime = 0;
  private frame: number | null = null;
  private clickTimer: unknown = null;
  private dragFadeTimer: unknown = null;
  private pending: ((arrived: boolean) => void) | null = null;
  private disposed = false;
  private frames = 0;
  private readonly listeners = new Set<(state: CursorState) => void>();

  constructor(options: AgentCursorControllerOptions) {
    this.el = options.elements;
    this.scheduler = options.scheduler ?? rafScheduler;
    this.timers = options.timers ?? defaultTimers;
    this.doc = options.document ?? (typeof document === "undefined" ? null : document);
    this.springConfig = options.spring ?? AGENT_CURSOR_SPRING;
    this.tier = options.tier ?? "full";
    this.reducedMotion = options.reducedMotion ?? false;
    this.doc?.addEventListener("visibilitychange", this.onVisibility);
    this.applyEnvironment();
    this.writeState();
    this.render(this.scheduler.now());
  }

  // ── Public API ────────────────────────────────────────────────────────

  get state(): CursorState {
    return this.current;
  }

  /** Displayed hotspot position (px, overlay space). */
  get position(): CursorPoint {
    return { x: this.x.position, y: this.y.position };
  }

  /** True while the rAF loop is scheduled. */
  get isAnimating(): boolean {
    return this.frame !== null;
  }

  /** Frames rendered by the loop since creation (diagnostics / perf readout). */
  get frameCount(): number {
    return this.frames;
  }

  subscribe(listener: (state: CursorState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Glide to (x, y). Resolves true on arrival, false if interrupted. */
  moveTo(x: number, y: number): Promise<boolean> {
    if (this.disposed) return Promise.resolve(false);
    const point = { x: finite(x), y: finite(y) };
    if (this.current === "hidden") {
      this.jumpTo(point);
      return Promise.resolve(false);
    }
    this.send("move");
    return this.travelTo(point);
  }

  /** Brief 170 ms ripple/refraction at the hotspot. */
  click(): void {
    if (this.disposed || !canTransition(this.current, "click")) return;
    this.send("click");
    const { x, y } = this.position;
    const at = `translate3d(${x}px, ${y}px, 0)`;
    if (this.reducedMotion) {
      play(this.el.ripple, [
        { transform: `${at} scale(1)`, opacity: 0.85 },
        { transform: `${at} scale(1)`, opacity: 0 },
      ], CLICK_MS);
    } else {
      play(this.el.ripple, [
        { transform: `${at} scale(0.35)`, opacity: 0.9 },
        { transform: `${at} scale(1.6)`, opacity: 0 },
      ], CLICK_MS);
      play(this.el.pointerBody, [{ transform: "scale(0.86)" }, { transform: "scale(1)" }], CLICK_MS);
    }
    if (this.clickTimer !== null) this.timers.clearTimeout(this.clickTimer);
    this.clickTimer = this.timers.setTimeout(() => {
      this.clickTimer = null;
      this.send("clickEnd");
    }, CLICK_MS);
  }

  /** Press-and-drag from the current position to (x, y), drawing a fading direction path. */
  dragTo(x: number, y: number): Promise<boolean> {
    if (this.disposed || !canTransition(this.current, "dragStart")) return Promise.resolve(false);
    this.send("dragStart");
    this.dragFrom = this.position;
    if (this.dragFadeTimer !== null) this.timers.clearTimeout(this.dragFadeTimer);
    this.dragFadeTimer = null;
    this.el.dragPath.style.opacity = "1";
    return this.travelTo({ x: finite(x), y: finite(y) });
  }

  typing(on: boolean): void {
    if (this.disposed) return;
    this.send(on ? "typeStart" : "typeEnd");
  }

  /** Become still and show attention (subtle ring; no bouncing). */
  wait(): void {
    if (this.disposed) return;
    this.send("wait");
  }

  /** Human took control (or no agent action): disappear immediately. */
  hide(): void {
    if (this.disposed) return;
    this.send("hide");
    this.settleNow();
  }

  show(): void {
    if (this.disposed) return;
    this.send("show");
  }

  setTier(tier: EffectsTier): void {
    this.tier = tier;
    this.applyEnvironment();
  }

  setReducedMotion(reducedMotion: boolean): void {
    this.reducedMotion = reducedMotion;
    this.applyEnvironment();
    if (reducedMotion) this.settleNow();
  }

  dispose(): void {
    if (this.disposed) return;
    this.stopLoop();
    this.resolvePending(false);
    if (this.clickTimer !== null) this.timers.clearTimeout(this.clickTimer);
    if (this.dragFadeTimer !== null) this.timers.clearTimeout(this.dragFadeTimer);
    this.doc?.removeEventListener("visibilitychange", this.onVisibility);
    this.listeners.clear();
    this.disposed = true;
  }

  // ── Internals ─────────────────────────────────────────────────────────

  private send(event: CursorEvent): void {
    const next = transitionCursor(this.current, event);
    if (next === this.current) return;
    const prev = this.current;
    this.current = next;
    if (prev === "dragging" && next !== "dragging") this.endDragPath();
    this.writeState();
    for (const listener of this.listeners) listener(next);
  }

  private travelTo(point: CursorPoint): Promise<boolean> {
    this.resolvePending(false);
    this.target = point;
    const done = new Promise<boolean>((resolve) => {
      this.pending = resolve;
    });
    if (this.reducedMotion || this.documentHidden()) {
      // No travel: appear at the destination with a short opacity fade.
      this.x = { position: point.x, velocity: 0 };
      this.y = { position: point.y, velocity: 0 };
      this.render(this.scheduler.now());
      if (this.reducedMotion) play(this.el.pointer, [{ opacity: 0 }, { opacity: 1 }], RM_FADE_MS);
      this.arrive();
      return done;
    }
    this.startLoop();
    return done;
  }

  private jumpTo(point: CursorPoint): void {
    this.target = point;
    this.x = { position: point.x, velocity: 0 };
    this.y = { position: point.y, velocity: 0 };
    this.history = [];
    this.render(this.scheduler.now());
  }

  private arrive(): void {
    if (this.current === "moving") this.send("arrive");
    else if (this.current === "dragging") this.send("dragEnd");
    this.resolvePending(true);
  }

  private resolvePending(arrived: boolean): void {
    const pending = this.pending;
    this.pending = null;
    pending?.(arrived);
  }

  private startLoop(): void {
    if (this.frame !== null || this.disposed) return;
    this.lastTime = this.scheduler.now();
    this.el.pointer.style.willChange = "transform";
    this.frame = this.scheduler.request(this.tick);
  }

  private stopLoop(): void {
    if (this.frame !== null) this.scheduler.cancel(this.frame);
    this.frame = null;
    this.el.pointer.style.willChange = "";
  }

  private readonly tick = (now: number): void => {
    this.frame = null;
    this.frames += 1;
    const dt = Math.min(Math.max((now - this.lastTime) / 1000, 0), MAX_DT_S);
    this.lastTime = now;
    const settledBefore = isSettled(this.x, this.target.x) && isSettled(this.y, this.target.y);
    if (!settledBefore) {
      this.x = stepSpring(this.x, this.target.x, this.springConfig, dt);
      this.y = stepSpring(this.y, this.target.y, this.springConfig, dt);
      this.lastMotion = now;
    }
    const settled = isSettled(this.x, this.target.x) && isSettled(this.y, this.target.y);
    if (settled) {
      this.x = { position: this.target.x, velocity: 0 };
      this.y = { position: this.target.y, velocity: 0 };
    }
    this.render(now);
    if (settled && this.pending) this.arrive();
    const trailBusy = trailLengthFor(this.tier, this.reducedMotion) > 0 && now - this.lastMotion <= this.trailSpan();
    if (!settled || trailBusy) {
      this.frame = this.scheduler.request(this.tick);
    } else {
      this.el.pointer.style.willChange = "";
    }
  };

  /** Jump to the end state without a loop (hidden document, hide, reduced motion). */
  private settleNow(): void {
    this.stopLoop();
    this.x = { position: this.target.x, velocity: 0 };
    this.y = { position: this.target.y, velocity: 0 };
    this.history = [];
    this.lastMotion = -Infinity;
    this.render(this.scheduler.now());
    if (this.pending) this.arrive();
  }

  private trailSpan(): number {
    return TRAIL_LAG_MS * (trailLengthFor(this.tier, this.reducedMotion) + 1);
  }

  private documentHidden(): boolean {
    return this.doc?.visibilityState === "hidden";
  }

  private readonly onVisibility = (): void => {
    if (this.documentHidden()) this.settleNow();
  };

  private render(now: number): void {
    const px = this.x.position;
    const py = this.y.position;
    this.el.pointer.style.transform = `translate3d(${px}px, ${py}px, 0)`;
    this.renderTrail(now, px, py);
    if (this.current === "dragging" && this.dragFrom) this.renderDragPath(this.dragFrom, { x: px, y: py });
  }

  private renderTrail(now: number, px: number, py: number): void {
    const count = trailLengthFor(this.tier, this.reducedMotion);
    if (count === 0) return;
    this.history.push({ t: now, x: px, y: py });
    if (this.history.length > HISTORY) this.history.shift();
    const speed = Math.hypot(this.x.velocity, this.y.velocity);
    const energy = Math.min(1, speed / TRAIL_FULL_SPEED);
    for (let i = 0; i < count; i += 1) {
      const node = this.el.trail[i];
      if (!node) continue;
      const p = this.sampleAt(now - TRAIL_LAG_MS * (i + 1), px, py);
      const fade = 1 - (i + 1) / (count + 1);
      const scale = 1 - (i + 1) * (0.5 / (count + 1));
      node.style.transform = `translate3d(${p.x}px, ${p.y}px, 0) scale(${scale.toFixed(3)})`;
      node.style.opacity = (0.55 * fade * energy).toFixed(3);
    }
  }

  private sampleAt(t: number, px: number, py: number): CursorPoint {
    const h = this.history;
    if (h.length === 0) return { x: px, y: py };
    const first = h[0] as Sample;
    if (t <= first.t) return { x: first.x, y: first.y };
    for (let i = h.length - 1; i > 0; i -= 1) {
      const a = h[i - 1] as Sample;
      const b = h[i] as Sample;
      if (t >= a.t && t <= b.t) {
        const k = b.t === a.t ? 1 : (t - a.t) / (b.t - a.t);
        return { x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k };
      }
    }
    return { x: px, y: py };
  }

  private renderDragPath(from: CursorPoint, to: CursorPoint): void {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const len = Math.hypot(dx, dy);
    const angle = Math.atan2(dy, dx);
    this.el.dragPath.style.transform = `translate3d(${from.x}px, ${from.y}px, 0) rotate(${angle}rad) scaleX(${Math.max(len, 0.001)})`;
  }

  private endDragPath(): void {
    this.dragFrom = null;
    if (this.dragFadeTimer !== null) this.timers.clearTimeout(this.dragFadeTimer);
    // Let the finished path read for a beat, then fade (CSS opacity transition).
    this.dragFadeTimer = this.timers.setTimeout(() => {
      this.dragFadeTimer = null;
      this.el.dragPath.style.opacity = "0";
    }, DRAG_FADE_DELAY_MS);
  }

  private applyEnvironment(): void {
    const count = trailLengthFor(this.tier, this.reducedMotion);
    this.el.trail.forEach((node, i) => {
      node.style.display = i < count ? "" : "none";
      if (i >= count) node.style.opacity = "0";
    });
    this.el.root.setAttribute("data-tier", this.tier);
    this.el.root.setAttribute("data-rm", this.reducedMotion ? "true" : "false");
  }

  private writeState(): void {
    this.el.root.setAttribute("data-state", this.current);
  }
}

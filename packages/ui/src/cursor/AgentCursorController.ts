/**
 * AgentCursorController — imperative, React-free driver of THE AGENT's
 * cursor in the computer preview. A visualization of actions that already
 * happened in the guest: it never feeds input anywhere and nothing waits
 * for it.
 *
 * Behaviour:
 * - Every action targets a NORMALIZED guest point; the frame rectangle
 *   (set by the layer from its size and the guest frame size) maps it to
 *   overlay pixels on every render, so a resize mid-glide stays exact.
 * - One glide at a time. A new action starts a fresh glide from the
 *   position on screen right now (see glide.ts); an effect still waiting
 *   for the old glide to land (a click pulse) plays at once, at its own real
 *   coordinate. Nothing is ever queued behind the newest action.
 * - `stop()` (Stop, task cancelled or failed, the person taking control,
 *   the computer stopping or being replaced) cancels motion and effects in
 *   the same call and forgets the position.
 *
 * Performance: positions never touch React. The rAF loop runs only while a
 * glide is in flight and writes `transform` on two nodes; pulses and cues
 * are Web Animations (compositor). No layout is read here: the layer
 * measures with a ResizeObserver.
 */
import type { EffectsTier } from "../tokens/effects.js";
import { easing } from "../tokens/motion.js";
import { DRAG_APPROACH_MAX_MS, dragDuration, glideDuration, sampleGlide, startGlide, type Glide } from "./glide.js";
import { clampUnit, snapToDevice, toOverlay, type Point, type Rect } from "./mapping.js";

export const CURSOR_STATES = [
  "hidden",
  "moving",
  "clicking",
  "dragging",
  "scrolling",
  "typing",
  "observing",
  "thinking",
  "done",
] as const;
export type CursorState = (typeof CURSOR_STATES)[number];

export interface FrameScheduler {
  request(callback: (now: number) => void): number;
  cancel(handle: number): void;
  now(): number;
}

export interface AgentCursorElements {
  /** Overlay root; receives data-state / data-pressed / data-tier / data-rm. */
  readonly root: HTMLElement;
  /** Positioned so its origin is the pointer's hotspot. */
  readonly pointer: HTMLElement;
  /** Inner node for press feedback (scale), so it never fights the position. */
  readonly pointerBody: HTMLElement;
  /** Two rings: a click uses the first, a double click both. */
  readonly pulses: readonly [HTMLElement, HTMLElement];
  /** 1 px-wide line, origin at its left-middle, scaled to the drag. */
  readonly dragPath: HTMLElement;
  /** Small directional mark shown beside the pointer on a scroll. */
  readonly scrollCue: HTMLElement;
}

export interface AgentCursorControllerOptions {
  readonly elements: AgentCursorElements;
  readonly tier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly scheduler?: FrameScheduler;
  readonly document?: Document;
  readonly devicePixelRatio?: () => number;
}

/** Click pulse: a soft ring, ~220 ms. */
export const PULSE_MS = 220;
/** A double click is two pulses this far apart. */
export const DOUBLE_GAP_MS = 110;
/** Reduced motion: the ring only fades, no growth. */
export const RM_PULSE_MS = 160;
export const SCROLL_CUE_MS = 420;
export const PRESS_MS = 160;

type Effect =
  | { readonly kind: "pulse"; readonly at: Point; readonly count: 1 | 2 }
  | { readonly kind: "scroll"; readonly at: Point; readonly dx: number; readonly dy: number };

interface DragLeg {
  readonly to: Point;
  readonly durationMs: number;
}

const ZERO: Point = { x: 0, y: 0 };
const EMPTY: Rect = { x: 0, y: 0, width: 0, height: 0 };

const rafScheduler: FrameScheduler = {
  request: (cb) => requestAnimationFrame(cb),
  cancel: (h) => cancelAnimationFrame(h),
  now: () => performance.now(),
};

function unit(p: Point): Point {
  return { x: clampUnit(p.x), y: clampUnit(p.y) };
}

function play(el: HTMLElement, frames: Keyframe[], options: KeyframeAnimationOptions): void {
  if (typeof el.animate === "function") el.animate(frames, { fill: "none", ...options });
}

function cancelAnimations(el: HTMLElement): void {
  if (typeof el.getAnimations === "function") for (const a of el.getAnimations()) a.cancel();
}

export class AgentCursorController {
  private readonly el: AgentCursorElements;
  private readonly scheduler: FrameScheduler;
  private readonly doc: Document | null;
  private readonly dpr: () => number;
  private reducedMotion: boolean;
  private current: CursorState = "hidden";
  private frameRect: Rect = EMPTY;
  /** Normalized position on screen; null until the first action (and after stop). */
  private pos: Point | null = null;
  private glide: Glide | null = null;
  private dragLeg: DragLeg | null = null;
  private dragFrom: Point | null = null;
  private pending: Effect | null = null;
  private pressed = false;
  private frame: number | null = null;
  private frames = 0;
  private disposed = false;
  private readonly listeners = new Set<(state: CursorState) => void>();

  constructor(options: AgentCursorControllerOptions) {
    this.el = options.elements;
    this.scheduler = options.scheduler ?? rafScheduler;
    this.doc = options.document ?? (typeof document === "undefined" ? null : document);
    this.dpr = options.devicePixelRatio ?? (() => (typeof window === "undefined" ? 1 : window.devicePixelRatio || 1));
    this.reducedMotion = options.reducedMotion ?? false;
    this.doc?.addEventListener("visibilitychange", this.onVisibility);
    this.setTier(options.tier ?? "full");
    this.setReducedMotion(this.reducedMotion);
    this.writeState();
    this.render();
  }

  // ── Diagnostics ─────────────────────────────────────────────────────

  get state(): CursorState {
    return this.current;
  }

  /** Normalized position on screen now (null before the first action). */
  get normalizedPosition(): Point | null {
    const glide = this.glide;
    if (!glide) return this.pos;
    return sampleGlide(glide, this.scheduler.now()).position;
  }

  /** Hotspot in overlay px (as last rendered). */
  get position(): Point {
    return this.pos ? toOverlay(this.pos, this.frameRect) : ZERO;
  }

  /** Where the current glide ends (normalized), if one is in flight. */
  get target(): Point | null {
    return this.glide?.to ?? null;
  }

  get isAnimating(): boolean {
    return this.frame !== null;
  }

  /** Visual work waiting behind the current glide: never more than one effect plus a drag's second leg. */
  get backlog(): number {
    return (this.pending ? 1 : 0) + (this.dragLeg ? 1 : 0);
  }

  get frameCount(): number {
    return this.frames;
  }

  get frameRectangle(): Rect {
    return this.frameRect;
  }

  subscribe(listener: (state: CursorState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  // ── Actions (each one is a real agent action that already happened) ──

  moveTo(point: Point): void {
    if (!this.begin()) return;
    this.setState("moving");
    this.travel(unit(point));
  }

  click(point: Point, count: 1 | 2 = 1): void {
    if (!this.begin()) return;
    this.setState("clicking");
    const at = unit(point);
    this.pending = { kind: "pulse", at, count };
    this.travel(at);
  }

  /** mouse_down: the pointer is held from here on (a pulse marks the press). */
  press(point: Point): void {
    if (!this.begin()) return;
    this.setState("clicking");
    const at = unit(point);
    this.setPressed(true);
    this.pending = { kind: "pulse", at, count: 1 };
    this.travel(at);
  }

  /** mouse_up: travel there and let go. */
  release(point: Point): void {
    if (!this.begin()) return;
    this.setState("moving");
    this.travel(unit(point));
    this.setPressed(false);
  }

  /**
   * Press at `from`, move continuously to `to` at the real drag's pace
   * (`durationMs`, bounded), release. A faint path shows the stroke and
   * fades as soon as it ends.
   */
  drag(from: Point, to: Point, durationMs: number): void {
    if (!this.begin()) return;
    this.setState("dragging");
    const start = unit(from);
    this.dragLeg = { to: unit(to), durationMs: dragDuration(durationMs, this.motionReduced()) };
    this.travel(start, DRAG_APPROACH_MAX_MS);
  }

  scroll(point: Point, dx: number, dy: number): void {
    if (!this.begin()) return;
    this.setState("scrolling");
    const at = unit(point);
    this.pending = { kind: "scroll", at, dx: Number.isFinite(dx) ? dx : 0, dy: Number.isFinite(dy) ? dy : 0 };
    this.travel(at);
  }

  // ── State only (no motion; a glide in flight finishes normally) ──────

  typing(on: boolean): void {
    if (this.disposed || this.current === "hidden") return;
    this.setState(on ? "typing" : "thinking");
  }

  /** The agent is looking at its screen: visible but subdued. */
  observe(): void {
    if (this.disposed || this.current === "hidden") return;
    this.setState("observing");
  }

  /** Between actions (the planner is thinking, a wait): quieter. */
  think(): void {
    if (this.disposed || this.current === "hidden") return;
    this.setState("thinking");
  }

  /** The task finished: settle where the last action was, unobtrusive. */
  done(): void {
    if (this.disposed || this.current === "hidden") return;
    this.setState("done");
  }

  /**
   * Stop, reset, cancellation, failure, the person taking control, the
   * computer stopping or being replaced: everything visual ends now.
   */
  stop(): void {
    if (this.disposed) return;
    this.stopLoop();
    this.glide = null;
    this.dragLeg = null;
    this.pending = null;
    this.pos = null;
    this.endDragPath(false);
    this.setPressed(false);
    for (const node of [...this.el.pulses, this.el.scrollCue, this.el.pointerBody]) cancelAnimations(node);
    this.setState("hidden");
  }

  // ── Environment ──────────────────────────────────────────────────────

  /** The guest frame's rectangle inside the overlay (px). */
  setFrame(rect: Rect): void {
    if (this.disposed) return;
    this.frameRect = rect;
    this.render();
  }

  setTier(tier: EffectsTier): void {
    this.el.root.setAttribute("data-tier", tier);
  }

  setReducedMotion(reducedMotion: boolean): void {
    this.reducedMotion = reducedMotion;
    this.el.root.setAttribute("data-rm", reducedMotion ? "true" : "false");
    if (reducedMotion) this.finishNow(true);
  }

  dispose(): void {
    if (this.disposed) return;
    this.stop();
    this.doc?.removeEventListener("visibilitychange", this.onVisibility);
    this.listeners.clear();
    this.disposed = true;
  }

  // ── Internals ────────────────────────────────────────────────────────

  /** A new action: whatever the previous one still owed plays now, then it is gone. */
  private begin(): boolean {
    if (this.disposed) return false;
    const effect = this.pending;
    this.pending = null;
    if (effect) this.playEffect(effect);
    if (this.dragLeg || this.dragFrom) {
      this.dragLeg = null;
      this.endDragPath(true);
      this.setPressed(false);
    }
    return true;
  }

  private motionReduced(): boolean {
    return this.reducedMotion || this.doc?.visibilityState === "hidden";
  }

  private travel(to: Point, maxMs = Infinity): void {
    const now = this.scheduler.now();
    if (this.pos === null) {
      // First sight: appear where the action happened, no travel from nowhere.
      this.pos = to;
      this.glide = null;
      this.render();
      this.arrive();
      return;
    }
    const here = this.glide ? sampleGlide(this.glide, now) : { position: this.pos, velocity: ZERO };
    const distance = Math.hypot((to.x - here.position.x) * this.frameRect.width, (to.y - here.position.y) * this.frameRect.height);
    const duration = Math.min(glideDuration(distance, this.motionReduced()), maxMs);
    this.pos = here.position;
    if (duration <= 0) {
      this.glide = null;
      this.pos = to;
      this.stopLoop();
      this.render();
      this.arrive();
      return;
    }
    this.glide = startGlide(here.position, to, here.velocity, now, duration);
    this.startLoop();
  }

  /** The glide landed (or there was none). */
  private arrive(): void {
    const leg = this.dragLeg;
    if (leg && this.pos) {
      this.dragLeg = null;
      this.dragFrom = this.pos;
      this.setPressed(true);
      this.el.dragPath.style.opacity = "1";
      if (leg.durationMs <= 0) {
        this.pos = leg.to;
        this.render();
        this.arrive();
        return;
      }
      this.glide = startGlide(this.pos, leg.to, ZERO, this.scheduler.now(), leg.durationMs);
      this.startLoop();
      return;
    }
    if (this.dragFrom) {
      this.endDragPath(true);
      this.setPressed(false);
    }
    const effect = this.pending;
    this.pending = null;
    if (effect) this.playEffect(effect);
  }

  private startLoop(): void {
    if (this.frame !== null || this.disposed) return;
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
    const glide = this.glide;
    if (!glide) return;
    const sample = sampleGlide(glide, now);
    this.pos = sample.position;
    if (sample.done) {
      this.glide = null;
      this.el.pointer.style.willChange = "";
      this.render();
      this.arrive();
      return;
    }
    this.render();
    this.frame = this.scheduler.request(this.tick);
  };

  /** Land immediately (reduced motion switched on, window hidden). */
  private finishNow(playEffects: boolean): void {
    if (!this.glide && !this.dragLeg) return;
    this.stopLoop();
    if (this.glide) this.pos = this.glide.to;
    this.glide = null;
    if (this.dragLeg) this.pos = this.dragLeg.to;
    this.dragLeg = null;
    if (!playEffects) this.pending = null;
    this.render();
    this.arrive();
  }

  private readonly onVisibility = (): void => {
    if (this.doc?.visibilityState === "hidden") this.finishNow(false);
  };

  private render(): void {
    if (!this.pos) return;
    const moving = this.glide !== null;
    const p = toOverlay(this.pos, this.frameRect);
    const dpr = this.dpr();
    const x = moving ? p.x : snapToDevice(p.x, dpr);
    const y = moving ? p.y : snapToDevice(p.y, dpr);
    this.el.pointer.style.transform = `translate3d(${x}px, ${y}px, 0)`;
    if (this.dragFrom) this.renderDragPath(toOverlay(this.dragFrom, this.frameRect), p);
  }

  private renderDragPath(from: Point, to: Point): void {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const length = Math.max(Math.hypot(dx, dy), 0.001);
    this.el.dragPath.style.transform = `translate3d(${from.x}px, ${from.y}px, 0) rotate(${Math.atan2(dy, dx)}rad) scaleX(${length})`;
  }

  private endDragPath(fade: boolean): void {
    this.dragFrom = null;
    const path = this.el.dragPath;
    // CSS owns the fade (opacity transition); without it the path vanishes at once.
    path.style.transition = fade ? "" : "none";
    path.style.opacity = "0";
  }

  private playEffect(effect: Effect): void {
    const at = toOverlay(effect.at, this.frameRect);
    const place = `translate3d(${at.x}px, ${at.y}px, 0)`;
    if (effect.kind === "pulse") {
      for (let i = 0; i < effect.count; i += 1) {
        const ring = this.el.pulses[i as 0 | 1];
        cancelAnimations(ring);
        const delay = i * DOUBLE_GAP_MS;
        if (this.reducedMotion) {
          play(ring, [{ transform: place, opacity: 0.8 }, { transform: place, opacity: 0 }], { duration: RM_PULSE_MS, delay, easing: easing.out });
        } else {
          play(ring, [
            { transform: `${place} scale(${i === 0 ? 0.3 : 0.45})`, opacity: 0.95 },
            { transform: `${place} scale(${i === 0 ? 1 : 0.85})`, opacity: 0 },
          ], { duration: PULSE_MS, delay, easing: easing.out });
        }
      }
      if (!this.reducedMotion) {
        cancelAnimations(this.el.pointerBody);
        play(this.el.pointerBody, [{ transform: "scale(0.86)" }, { transform: "scale(1)" }], {
          duration: PRESS_MS,
          easing: easing.out,
          iterations: effect.count,
        });
      }
      return;
    }
    const cue = this.el.scrollCue;
    cancelAnimations(cue);
    const horizontal = Math.abs(effect.dx) > Math.abs(effect.dy);
    const sign = (horizontal ? effect.dx : effect.dy) < 0 ? -1 : 1;
    // The cue's chevron points down at 0deg.
    const angle = horizontal ? (sign > 0 ? -90 : 90) : sign > 0 ? 0 : 180;
    const shift = this.reducedMotion ? 0 : 4 * sign;
    const tx = horizontal ? shift : 0;
    const ty = horizontal ? 0 : shift;
    const base = `${place} translate(16px, 10px) rotate(${angle}deg)`;
    play(cue, [
      { transform: `${place} translate(16px, 10px) translate(${-tx}px, ${-ty}px) rotate(${angle}deg)`, opacity: 0 },
      { transform: base, opacity: 0.85, offset: 0.35 },
      { transform: `${place} translate(16px, 10px) translate(${tx}px, ${ty}px) rotate(${angle}deg)`, opacity: 0 },
    ], { duration: SCROLL_CUE_MS, easing: easing.out });
  }

  private setPressed(pressed: boolean): void {
    if (this.pressed === pressed) return;
    this.pressed = pressed;
    this.el.root.toggleAttribute("data-pressed", pressed);
  }

  private setState(next: CursorState): void {
    if (next === this.current) return;
    this.current = next;
    this.writeState();
    for (const listener of this.listeners) listener(next);
  }

  private writeState(): void {
    this.el.root.setAttribute("data-state", this.current);
  }
}

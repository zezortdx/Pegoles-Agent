/**
 * AmbientController — the single Idle-GPU gate.
 *
 * Writes three facts onto one root element and nothing else:
 *   data-effects-tier="full|reduced|minimal"
 *   data-reduced-motion="true|false"
 *   data-ambient="running|paused"   (+ data-ambient-reason for diagnostics)
 *   data-visibility="visible|hidden"
 *
 * CSS keys every continuous animation off these attributes
 * (`animation-play-state`), so pausing ambient life is one attribute
 * write: no React state, no rerender cascade, no JS animation loop.
 *
 * Ambient runs only when ALL hold: the tier allows ambient, the document
 * is visible, the window is focused, the host has not reported the window
 * hidden/minimized, and the app is not idle (idle = no input for
 * `idleAfterMs` AND no active work, i.e. `busy` is false).
 * Reduced motion is recorded, never altered, and does not pause the gate:
 * CSS swaps movement for static/opacity treatments under it.
 */
import { effectsTiers, type EffectsTier } from "../tokens/effects.js";

export type AmbientPauseReason = "stopped" | "tier" | "hidden" | "unfocused" | "idle";

export interface AmbientSnapshot {
  readonly running: boolean;
  /** Why ambient is paused, most fundamental first. Empty when running. */
  readonly reasons: readonly AmbientPauseReason[];
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  readonly visible: boolean;
  readonly focused: boolean;
  readonly idle: boolean;
  readonly busy: boolean;
  readonly externallyHidden: boolean;
}

export interface AmbientControllerOptions {
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  /** Element that receives the data attributes. Default: documentElement. */
  readonly root?: HTMLElement;
  readonly window?: Window;
  readonly document?: Document;
  /** No-input interval before the app counts as idle. Default 60 s. */
  readonly idleAfterMs?: number;
  /** Active work (task running, computer booting): never idle while true. */
  readonly busy?: boolean;
  /** Host signal, e.g. Tauri window minimized. */
  readonly externallyHidden?: boolean;
  /** Initial focus; default `document.hasFocus()`. */
  readonly initiallyFocused?: boolean;
}

export const DEFAULT_IDLE_AFTER_MS = 60_000;

const ACTIVITY_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

type Listener = (snapshot: AmbientSnapshot) => void;

export class AmbientController {
  private readonly win: Window;
  private readonly doc: Document;
  private readonly root: HTMLElement;
  private readonly idleAfterMs: number;
  private tier: EffectsTier;
  private reducedMotion: boolean;
  private busy: boolean;
  private externallyHidden: boolean;
  private focused: boolean;
  private visible: boolean;
  private idle = false;
  private started = false;
  private lastActivity = 0;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  private current: AmbientSnapshot;
  private readonly listeners = new Set<Listener>();

  constructor(options: AmbientControllerOptions) {
    this.win = options.window ?? window;
    this.doc = options.document ?? this.win.document;
    this.root = options.root ?? this.doc.documentElement;
    this.idleAfterMs = Math.max(1000, options.idleAfterMs ?? DEFAULT_IDLE_AFTER_MS);
    this.tier = options.tier;
    this.reducedMotion = options.reducedMotion;
    this.busy = options.busy ?? false;
    this.externallyHidden = options.externallyHidden ?? false;
    this.focused = options.initiallyFocused ?? this.doc.hasFocus();
    this.visible = this.doc.visibilityState !== "hidden";
    this.current = this.compute();
  }

  start(): void {
    if (this.started) return;
    this.started = true;
    this.win.addEventListener("focus", this.onFocus);
    this.win.addEventListener("blur", this.onBlur);
    this.doc.addEventListener("visibilitychange", this.onVisibility);
    for (const type of ACTIVITY_EVENTS) {
      this.win.addEventListener(type, this.onActivity, { passive: true, capture: true });
    }
    this.visible = this.doc.visibilityState !== "hidden";
    this.lastActivity = Date.now();
    this.idle = false;
    this.scheduleIdleCheck(this.idleAfterMs);
    this.commit();
  }

  stop(): void {
    if (!this.started) return;
    this.started = false;
    this.win.removeEventListener("focus", this.onFocus);
    this.win.removeEventListener("blur", this.onBlur);
    this.doc.removeEventListener("visibilitychange", this.onVisibility);
    for (const type of ACTIVITY_EVENTS) {
      this.win.removeEventListener(type, this.onActivity, { capture: true });
    }
    this.clearIdleTimer();
    this.commit();
  }

  setTier(tier: EffectsTier): void {
    if (tier === this.tier) return;
    this.tier = tier;
    this.commit();
  }

  setReducedMotion(reducedMotion: boolean): void {
    if (reducedMotion === this.reducedMotion) return;
    this.reducedMotion = reducedMotion;
    this.commit();
  }

  setBusy(busy: boolean): void {
    if (busy === this.busy) return;
    this.busy = busy;
    if (busy) {
      this.idle = false;
    } else {
      this.lastActivity = Date.now();
      this.scheduleIdleCheck(this.idleAfterMs);
    }
    this.commit();
  }

  setExternallyHidden(hidden: boolean): void {
    if (hidden === this.externallyHidden) return;
    this.externallyHidden = hidden;
    this.commit();
  }

  /** Record user activity from outside the DOM (e.g. native input). */
  notifyActivity(): void {
    this.onActivity();
  }

  snapshot(): AmbientSnapshot {
    return this.current;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private readonly onFocus = (): void => {
    this.focused = true;
    this.onActivity();
    this.commit();
  };

  private readonly onBlur = (): void => {
    this.focused = false;
    this.commit();
  };

  private readonly onVisibility = (): void => {
    this.visible = this.doc.visibilityState !== "hidden";
    this.commit();
  };

  /** Hot path (pointermove): a timestamp write, plus a wake-up if idle. */
  private readonly onActivity = (): void => {
    this.lastActivity = Date.now();
    if (this.idle) {
      this.idle = false;
      this.scheduleIdleCheck(this.idleAfterMs);
      this.commit();
    } else if (this.idleTimer === null && this.started) {
      this.scheduleIdleCheck(this.idleAfterMs);
    }
  };

  private scheduleIdleCheck(delay: number): void {
    this.clearIdleTimer();
    if (!this.started) return;
    this.idleTimer = setTimeout(this.checkIdle, delay);
  }

  private readonly checkIdle = (): void => {
    this.idleTimer = null;
    if (!this.started) return;
    if (this.busy) return; // resumes via setBusy(false)
    const quietFor = Date.now() - this.lastActivity;
    if (quietFor >= this.idleAfterMs) {
      this.idle = true;
      this.commit();
    } else {
      this.scheduleIdleCheck(this.idleAfterMs - quietFor);
    }
  };

  private clearIdleTimer(): void {
    if (this.idleTimer !== null) {
      clearTimeout(this.idleTimer);
      this.idleTimer = null;
    }
  }

  private compute(): AmbientSnapshot {
    const reasons: AmbientPauseReason[] = [];
    if (!this.started) reasons.push("stopped");
    if (!effectsTiers[this.tier].ambient.enabled) reasons.push("tier");
    if (!this.visible || this.externallyHidden) reasons.push("hidden");
    if (!this.focused) reasons.push("unfocused");
    if (this.idle && !this.busy) reasons.push("idle");
    return {
      running: reasons.length === 0,
      reasons,
      tier: this.tier,
      reducedMotion: this.reducedMotion,
      visible: this.visible && !this.externallyHidden,
      focused: this.focused,
      idle: this.idle && !this.busy,
      busy: this.busy,
      externallyHidden: this.externallyHidden,
    };
  }

  private commit(): void {
    const next = this.compute();
    this.writeAttributes(next);
    const prev = this.current;
    this.current = next;
    if (
      prev.running !== next.running ||
      prev.reasons.join() !== next.reasons.join() ||
      prev.tier !== next.tier ||
      prev.reducedMotion !== next.reducedMotion ||
      prev.visible !== next.visible ||
      prev.focused !== next.focused ||
      prev.idle !== next.idle ||
      prev.busy !== next.busy
    ) {
      for (const listener of this.listeners) listener(next);
    }
  }

  /** Writes only attributes whose value changed (avoids style recalcs). */
  private writeAttributes(s: AmbientSnapshot): void {
    const set = (name: string, value: string): void => {
      if (this.root.getAttribute(name) !== value) this.root.setAttribute(name, value);
    };
    set("data-effects-tier", s.tier);
    set("data-reduced-motion", s.reducedMotion ? "true" : "false");
    set("data-ambient", s.running ? "running" : "paused");
    set("data-ambient-reason", s.reasons[0] ?? "none");
    set("data-visibility", s.visible ? "visible" : "hidden");
  }
}

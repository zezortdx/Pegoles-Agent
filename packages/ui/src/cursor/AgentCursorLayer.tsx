import { forwardRef, useId, useImperativeHandle, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { EffectsTier } from "../tokens/effects.js";
import { useResolvedEffects } from "../presence/useResolvedEffects.js";
import {
  AgentCursorController,
  MAX_TRAIL_ELEMENTS,
  type CursorTimers,
  type FrameScheduler,
  type CursorPoint,
} from "./AgentCursorController.js";
import type { CursorState } from "./cursorMachine.js";
import "./cursor.css";

/** The imperative surface exposed through the layer's ref. */
export interface AgentCursorHandle {
  moveTo(x: number, y: number): Promise<boolean>;
  click(): void;
  dragTo(x: number, y: number): Promise<boolean>;
  typing(on: boolean): void;
  wait(): void;
  hide(): void;
  show(): void;
  readonly state: CursorState;
  readonly position: CursorPoint;
  readonly isAnimating: boolean;
  readonly frameCount: number;
  /** State changes only (rare) — never positions. */
  subscribe(listener: (state: CursorState) => void): () => void;
}

export interface AgentCursorLayerProps {
  readonly effectsTier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly className?: string;
  readonly style?: CSSProperties;
  /** Advanced/tests: custom frame clock (default requestAnimationFrame). */
  readonly frameScheduler?: FrameScheduler;
  /** Advanced/tests: custom timers (default setTimeout). */
  readonly timers?: CursorTimers;
}

const TRAIL_KEYS = Array.from({ length: MAX_TRAIL_ELEMENTS }, (_, i) => i);
const NOT_READY = Promise.resolve(false);

/** Luminous agent pointer; hotspot (tip) at the element origin. */
function PointerGlyph({ uid }: { uid: string }) {
  const d = "M0 0L0 15.6C0 16.7 1.3 17.2 2 16.4L5.6 12.6C5.9 12.3 6.3 12.1 6.8 12.1L12.2 12.1C13.3 12.1 13.8 10.8 13 10L1.9 -0.8C1.2 -1.5 0 -1 0 0Z";
  return (
    <svg className="pgc-glyph" viewBox="-5 -5 24 27" aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id={`${uid}-core`} x1="0" y1="0" x2="0.9" y2="1">
          <stop offset="0" stopColor="#F4F7FC" />
          <stop offset="0.45" stopColor="#97EBFD" />
          <stop offset="1" stopColor="#4CCEFC" />
        </linearGradient>
        <filter id={`${uid}-glow`} x="-60%" y="-60%" width="220%" height="220%">
          <feGaussianBlur stdDeviation="2.2" />
        </filter>
      </defs>
      <path d={d} fill="#169CFD" opacity="0.75" filter={`url(#${uid}-glow)`} />
      <path d={d} fill={`url(#${uid}-core)`} stroke="#169CFD" strokeWidth="1.3" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * Overlay for THE AGENT's cursor (never the human's pointer). Renders its
 * DOM once; all motion goes through the imperative handle (see
 * `AgentCursorController`) so moving it never re-renders React. The layer
 * is `pointer-events: none` and aria-hidden: agent actions are announced
 * by the activity feed, not by this decoration.
 */
export const AgentCursorLayer = forwardRef<AgentCursorHandle, AgentCursorLayerProps>(function AgentCursorLayer(
  { effectsTier, reducedMotion, className, style, frameScheduler, timers },
  ref,
) {
  const effects = useResolvedEffects(effectsTier, reducedMotion);
  const rootRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const rippleRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<HTMLDivElement>(null);
  const trailRefs = useRef<(HTMLDivElement | null)[]>([]);
  const controllerRef = useRef<AgentCursorController | null>(null);
  const uid = `pgc${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;

  // Stable facade: survives StrictMode remounts (the controller behind it may be recreated).
  const [facade] = useState<AgentCursorHandle>(() => ({
    moveTo: (x, y) => controllerRef.current?.moveTo(x, y) ?? NOT_READY,
    click: () => controllerRef.current?.click(),
    dragTo: (x, y) => controllerRef.current?.dragTo(x, y) ?? NOT_READY,
    typing: (on) => controllerRef.current?.typing(on),
    wait: () => controllerRef.current?.wait(),
    hide: () => controllerRef.current?.hide(),
    show: () => controllerRef.current?.show(),
    get state() {
      return controllerRef.current?.state ?? "hidden";
    },
    get position() {
      return controllerRef.current?.position ?? { x: 0, y: 0 };
    },
    get isAnimating() {
      return controllerRef.current?.isAnimating ?? false;
    },
    get frameCount() {
      return controllerRef.current?.frameCount ?? 0;
    },
    subscribe: (listener) => controllerRef.current?.subscribe(listener) ?? (() => undefined),
  }));
  useLayoutEffect(() => {
    const root = rootRef.current;
    const pointer = pointerRef.current;
    const pointerBody = bodyRef.current;
    const ripple = rippleRef.current;
    const dragPath = dragRef.current;
    if (!root || !pointer || !pointerBody || !ripple || !dragPath) return undefined;
    const controller = new AgentCursorController({
      elements: {
        root,
        pointer,
        pointerBody,
        ripple,
        dragPath,
        trail: trailRefs.current.filter((n): n is HTMLDivElement => n !== null),
      },
      scheduler: frameScheduler,
      timers,
    });
    controllerRef.current = controller;
    return () => {
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
    // The clock and timers are fixed for the layer's lifetime by design.
  }, []);

  useLayoutEffect(() => {
    controllerRef.current?.setTier(effects.tier);
  }, [effects.tier]);

  useLayoutEffect(() => {
    controllerRef.current?.setReducedMotion(effects.reducedMotion);
  }, [effects.reducedMotion]);

  // Declared after the controller effect so a parent's ref callback sees a live cursor.
  useImperativeHandle(ref, () => facade, [facade]);

  return (
    <div
      ref={rootRef}
      className={className ? `pgc-layer ${className}` : "pgc-layer"}
      style={style}
      aria-hidden="true"
    >
      <div ref={dragRef} className="pgc-drag" />
      {TRAIL_KEYS.map((i) => (
        <div
          key={i}
          className="pgc-trail"
          ref={(node) => {
            trailRefs.current[i] = node;
          }}
        />
      ))}
      <div ref={rippleRef} className="pgc-ripple" />
      <div ref={pointerRef} className="pgc-pointer">
        <div ref={bodyRef} className="pgc-body">
          <span className="pgc-wait">
            <i className="pg-ambient" />
          </span>
          <PointerGlyph uid={uid} />
          <span className="pgc-typing">
            <i className="pg-work-anim" />
            <i className="pg-work-anim" />
            <i className="pg-work-anim" />
          </span>
        </div>
      </div>
    </div>
  );
});

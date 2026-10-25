import { forwardRef, useId, useImperativeHandle, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { EffectsTier } from "../tokens/effects.js";
import { useResolvedEffects } from "../presence/useResolvedEffects.js";
import { AgentCursorController, type CursorState, type FrameScheduler } from "./AgentCursorController.js";
import { frameRect, type FrameFit, type Point, type Rect, type Size } from "./mapping.js";
import "./cursor.css";

/** The imperative surface exposed through the layer's ref (normalized guest points). */
export interface AgentCursorHandle {
  moveTo(point: Point): void;
  click(point: Point, count?: 1 | 2): void;
  press(point: Point): void;
  release(point: Point): void;
  drag(from: Point, to: Point, durationMs: number): void;
  scroll(point: Point, dx: number, dy: number): void;
  typing(on: boolean): void;
  observe(): void;
  think(): void;
  done(): void;
  stop(): void;
  readonly state: CursorState;
  /** Hotspot in overlay px. */
  readonly position: Point;
  readonly normalizedPosition: Point | null;
  readonly target: Point | null;
  readonly isAnimating: boolean;
  readonly backlog: number;
  readonly frameCount: number;
  readonly frameRectangle: Rect;
  /** State changes only (rare) — never positions. */
  subscribe(listener: (state: CursorState) => void): () => void;
}

export interface AgentCursorLayerProps {
  /**
   * Guest framebuffer size (px). The frame is drawn `object-fit: contain`
   * in the layer's box; without a size it fills the box.
   */
  readonly frameSize?: Size | null;
  /** How the preview draws the frame in this box (its CSS `object-fit`). Default contain. */
  readonly fit?: FrameFit;
  readonly effectsTier?: EffectsTier;
  readonly reducedMotion?: boolean;
  readonly className?: string;
  readonly style?: CSSProperties;
  /** Advanced/tests: custom frame clock (default requestAnimationFrame). */
  readonly frameScheduler?: FrameScheduler;
}

const IDLE: AgentCursorHandle["state"] = "hidden";

/**
 * The agent pointer: a small silver arrow with a dark hairline (legible on
 * light and dark guest content) and the mark's soft white glow. Hotspot =
 * the tip, at the element origin.
 */
function PointerGlyph({ uid }: { uid: string }) {
  const d = "M0.6 0.9L0.6 15.2C0.6 16.1 1.7 16.5 2.3 15.9L5.2 12.9L7.5 18.3C7.8 18.9 8.4 19.2 9 18.9L10 18.5C10.6 18.2 10.9 17.5 10.6 16.9L8.4 11.8L12.5 11.8C13.4 11.8 13.8 10.7 13.2 10.1L2.2 0.2C1.6 -0.3 0.6 0.1 0.6 0.9Z";
  return (
    <svg className="pgc-glyph" viewBox="-4 -4 22 27" aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id={`${uid}-fill`} x1="0.1" y1="0" x2="0.75" y2="1">
          <stop offset="0" stopColor="#FFFFFF" />
          <stop offset="0.55" stopColor="#EEF0F2" />
          <stop offset="1" stopColor="#C4C8CD" />
        </linearGradient>
      </defs>
      <path d={d} fill={`url(#${uid}-fill)`} stroke="#0B0C0E" strokeOpacity="0.82" strokeWidth="1.15" strokeLinejoin="round" />
      <path d="M2.1 3.2L2.1 13.2" stroke="#FFFFFF" strokeOpacity="0.9" strokeWidth="0.8" strokeLinecap="round" />
    </svg>
  );
}

/**
 * Overlay for THE AGENT's cursor (never the human's pointer). Renders its
 * DOM once; all motion goes through the imperative handle (see
 * `AgentCursorController`) so moving it never re-renders React. The layer
 * covers the preview box, is `pointer-events: none` and aria-hidden: the
 * activity feed announces actions, this only shows them.
 */
export const AgentCursorLayer = forwardRef<AgentCursorHandle, AgentCursorLayerProps>(function AgentCursorLayer(
  { frameSize = null, fit = "contain", effectsTier, reducedMotion, className, style, frameScheduler },
  ref,
) {
  const effects = useResolvedEffects(effectsTier, reducedMotion);
  const rootRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const pulseA = useRef<HTMLDivElement>(null);
  const pulseB = useRef<HTMLDivElement>(null);
  const dragRef = useRef<HTMLDivElement>(null);
  const cueRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<AgentCursorController | null>(null);
  const boxRef = useRef<Size>({ width: 0, height: 0 });
  const frameSizeRef = useRef<Size | null>(frameSize);
  const fitRef = useRef<FrameFit>(fit);
  const uid = `pgc${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;

  // Stable facade: survives StrictMode remounts (the controller behind it may be recreated).
  const [facade] = useState<AgentCursorHandle>(() => ({
    moveTo: (p) => controllerRef.current?.moveTo(p),
    click: (p, count) => controllerRef.current?.click(p, count),
    press: (p) => controllerRef.current?.press(p),
    release: (p) => controllerRef.current?.release(p),
    drag: (from, to, ms) => controllerRef.current?.drag(from, to, ms),
    scroll: (p, dx, dy) => controllerRef.current?.scroll(p, dx, dy),
    typing: (on) => controllerRef.current?.typing(on),
    observe: () => controllerRef.current?.observe(),
    think: () => controllerRef.current?.think(),
    done: () => controllerRef.current?.done(),
    stop: () => controllerRef.current?.stop(),
    get state() {
      return controllerRef.current?.state ?? IDLE;
    },
    get position() {
      return controllerRef.current?.position ?? { x: 0, y: 0 };
    },
    get normalizedPosition() {
      return controllerRef.current?.normalizedPosition ?? null;
    },
    get target() {
      return controllerRef.current?.target ?? null;
    },
    get isAnimating() {
      return controllerRef.current?.isAnimating ?? false;
    },
    get backlog() {
      return controllerRef.current?.backlog ?? 0;
    },
    get frameCount() {
      return controllerRef.current?.frameCount ?? 0;
    },
    get frameRectangle() {
      return controllerRef.current?.frameRectangle ?? { x: 0, y: 0, width: 0, height: 0 };
    },
    subscribe: (listener) => controllerRef.current?.subscribe(listener) ?? (() => undefined),
  }));

  useLayoutEffect(() => {
    const root = rootRef.current;
    const pointer = pointerRef.current;
    const pointerBody = bodyRef.current;
    const a = pulseA.current;
    const b = pulseB.current;
    const dragPath = dragRef.current;
    const scrollCue = cueRef.current;
    if (!root || !pointer || !pointerBody || !a || !b || !dragPath || !scrollCue) return undefined;
    const controller = new AgentCursorController({
      elements: { root, pointer, pointerBody, pulses: [a, b], dragPath, scrollCue },
      scheduler: frameScheduler,
    });
    controllerRef.current = controller;
    const place = () => controller.setFrame(frameRect(boxRef.current, frameSizeRef.current, fitRef.current));
    // The box is measured by the observer (no layout reads per frame).
    boxRef.current = { width: root.clientWidth, height: root.clientHeight };
    place();
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver((entries) => {
        const entry = entries[entries.length - 1];
        if (!entry) return;
        boxRef.current = { width: entry.contentRect.width, height: entry.contentRect.height };
        place();
      });
      observer.observe(root);
    }
    return () => {
      observer?.disconnect();
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
    // The clock is fixed for the layer's lifetime by design.
  }, []);

  useLayoutEffect(() => {
    frameSizeRef.current = frameSize;
    fitRef.current = fit;
    controllerRef.current?.setFrame(frameRect(boxRef.current, frameSize, fit));
  }, [frameSize?.width, frameSize?.height, fit]);

  useLayoutEffect(() => {
    controllerRef.current?.setTier(effects.tier);
  }, [effects.tier]);

  useLayoutEffect(() => {
    controllerRef.current?.setReducedMotion(effects.reducedMotion);
  }, [effects.reducedMotion]);

  // Declared after the controller effect so a parent's ref callback sees a live cursor.
  useImperativeHandle(ref, () => facade, [facade]);

  return (
    <div ref={rootRef} className={className ? `pgc-layer ${className}` : "pgc-layer"} style={style} data-state="hidden" aria-hidden="true">
      <div ref={dragRef} className="pgc-drag" />
      <div ref={pulseA} className="pgc-pulse" />
      <div ref={pulseB} className="pgc-pulse" />
      <div ref={cueRef} className="pgc-scroll">
        <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false">
          <path d="M3 3.2L6 6.2L9 3.2M3 6.4L6 9.4L9 6.4" />
        </svg>
      </div>
      <div ref={pointerRef} className="pgc-pointer">
        <div ref={bodyRef} className="pgc-body">
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

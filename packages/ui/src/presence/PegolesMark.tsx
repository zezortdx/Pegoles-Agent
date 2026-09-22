import { useId, type CSSProperties } from "react";
import { MARK_ARTBOARD, MARK_GEOMETRY, MARK_RING_MASK, MARK_RING_PATH } from "../brand/index.js";
import { palette } from "../tokens/palette.js";
import { useOffscreenPause } from "../runtime/offscreen.js";
import type { EffectsTier } from "../tokens/effects.js";
import { attentionAngle, eyeOffset, type Vec2 } from "./geometry.js";
import { PRESENCE_LABEL, type PresenceState } from "./presenceMachine.js";
import { presenceVisual, type AnimationTarget, type PresenceVisual } from "./presenceVisual.js";
import { useResolvedEffects } from "./useResolvedEffects.js";
import "./presence.css";

export interface PegolesMarkProps {
  /** Rendered size in CSS px (square box). Works from 20 (nav rail) to ~160 (hero). */
  readonly size?: number;
  /** Presence state — decided by the app from real state (see `usePresence`). */
  readonly state?: PresenceState;
  /** Effects tier; defaults to the Flux Glass root's tier. */
  readonly effectsTier?: EffectsTier;
  /** Reduced motion; defaults to the root / OS preference. */
  readonly reducedMotion?: boolean;
  /**
   * Direction (screen space, y down) toward relevant active content. The
   * eyes drift at most 3 px (less at small sizes), eased; ignored under
   * reduced motion and the Minimal tier.
   */
  readonly lookAt?: Vec2 | null;
  /**
   * Where the command surface is, for Listening / Waiting light
   * concentration: degrees (0 = right, 90 = down) or a direction vector.
   * Default: down.
   */
  readonly attention?: number | Vec2 | null;
  /** Accessible name prefix. Default "Pegoles". */
  readonly label?: string;
  /** Hide from assistive tech (when adjacent visible text already names it). */
  readonly decorative?: boolean;
  readonly className?: string;
  readonly style?: CSSProperties;
}

const { paths, colors, look, eyes } = MARK_GEOMETRY;
const EYES_PATH = `${paths.eyeLeft}${paths.eyeRight}`;
const EYE_TOP = Math.min(...eyes.map((e) => e.cy - e.height / 2));
const EYE_BOTTOM = Math.max(...eyes.map((e) => e.cy + e.height / 2));
const RING_TOP = MARK_GEOMETRY.bounds.y;
const RING_BOTTOM = MARK_GEOMETRY.bounds.y + MARK_GEOMETRY.bounds.height;
const OPENING_CY = MARK_GEOMETRY.opening.y + MARK_GEOMETRY.opening.height / 2;
const A = MARK_ARTBOARD;
/** Glow layer bleeds one artboard-half beyond the box on every side. */
const GLOW_BOX = `${-A / 2} ${-A / 2} ${A * 2} ${A * 2}`;
/** Radius (fraction of size) at which the attention light sits on the ring. */
const ATTENTION_RADIUS = 0.44;
const GLOW_LEAN = 0.035;
const RING_MASK_STYLE: CSSProperties = { maskImage: MARK_RING_MASK, WebkitMaskImage: MARK_RING_MASK };

type Stops = readonly { readonly offset: number; readonly color: string }[];

function Gradient({ id, stops, y1, y2 }: { id: string; stops: Stops; y1: number; y2: number }) {
  return (
    <linearGradient id={id} gradientUnits="userSpaceOnUse" x1="0" y1={y1} x2="0" y2={y2}>
      {stops.map((s) => (
        <stop key={s.offset} offset={s.offset} stopColor={s.color} />
      ))}
    </linearGradient>
  );
}

function Blur({ id, std }: { id: string; std: number }) {
  return (
    <filter id={id} filterUnits="userSpaceOnUse" x={-A / 2} y={-A / 2} width={A * 2} height={A * 2}>
      <feGaussianBlur stdDeviation={std} />
    </filter>
  );
}

function animationStyle(visual: PresenceVisual, target: AnimationTarget): CSSProperties | undefined {
  const anim = visual.animations.find((a) => a.target === target);
  if (!anim) return undefined;
  // Longhands only: play-state stays owned by the gate CSS (.pg-ambient /
  // .pg-work-anim), which pauses loops when idle, hidden or offscreen.
  return {
    animationName: anim.keyframes,
    animationDuration: `${anim.durationMs}ms`,
    animationTimingFunction: anim.easing,
    animationIterationCount: anim.iterations === "infinite" ? "infinite" : String(anim.iterations),
    animationDirection: anim.direction,
    animationFillMode: anim.gate === "once" ? "both" : "none",
  };
}

function gateClass(visual: PresenceVisual, target: AnimationTarget): string {
  const anim = visual.animations.find((a) => a.target === target);
  if (!anim) return "";
  if (anim.gate === "ambient") return " pg-ambient";
  if (anim.gate === "work") return " pg-work-anim";
  return "";
}

function GlowLayer({ uid, visual, shift }: { uid: string; visual: PresenceVisual; shift: Vec2 }) {
  return (
    <div
      className="pgm-layer pgm-glow"
      style={{
        opacity: visual.levels.glow,
        transform: shift.x || shift.y ? `translate3d(${shift.x}px, ${shift.y}px, 0)` : undefined,
      }}
    >
      <div className={`pgm-fill${gateClass(visual, "glow")}`} style={animationStyle(visual, "glow")}>
        <svg className="pgm-glow-svg" viewBox={GLOW_BOX} aria-hidden="true" focusable="false">
          <defs>
            {visual.layers.farGlow ? <Blur id={`${uid}-gf`} std={look.glow_far_std} /> : null}
            <Blur id={`${uid}-gn`} std={look.glow_near_std} />
            <Blur id={`${uid}-gt`} std={look.glow_tight_std} />
          </defs>
          {visual.layers.farGlow ? (
            <path
              d={MARK_RING_PATH}
              fillRule="evenodd"
              fill={colors.glow}
              opacity={look.glow_far_opacity}
              filter={`url(#${uid}-gf)`}
            />
          ) : null}
          <path
            d={MARK_RING_PATH}
            fillRule="evenodd"
            fill={colors.glow}
            opacity={look.glow_near_opacity}
            filter={`url(#${uid}-gn)`}
          />
          <path
            d={MARK_RING_PATH}
            fillRule="evenodd"
            fill={colors.glow}
            opacity={look.glow_tight_opacity}
            filter={`url(#${uid}-gt)`}
          />
        </svg>
      </div>
    </div>
  );
}

function RingLayer({ uid, visual }: { uid: string; visual: PresenceVisual }) {
  const detailed = !visual.compact;
  return (
    <svg className="pgm-layer pgm-ring" viewBox={`0 0 ${A} ${A}`} aria-hidden="true" focusable="false">
      <defs>
        <Gradient id={`${uid}-body`} stops={colors.ringBody} y1={RING_TOP} y2={RING_BOTTOM} />
        <Gradient id={`${uid}-light`} stops={colors.ringLight} y1={RING_TOP} y2={RING_BOTTOM} />
        <Gradient id={`${uid}-rim`} stops={colors.ringRim} y1={RING_TOP} y2={RING_BOTTOM} />
        <Gradient id={`${uid}-edge`} stops={colors.ringInnerEdge} y1={RING_TOP} y2={RING_BOTTOM} />
        <clipPath id={`${uid}-ring`}>
          <path d={MARK_RING_PATH} clipRule="evenodd" />
        </clipPath>
        <clipPath id={`${uid}-open`}>
          <path d={paths.inner} />
        </clipPath>
        {detailed ? (
          <>
            <Blur id={`${uid}-soft`} std={look.light_std} />
            <Blur id={`${uid}-fine`} std={look.rim_std} />
            <Blur id={`${uid}-shade`} std={look.shade_std} />
          </>
        ) : null}
      </defs>
      {detailed ? (
        <g clipPath={`url(#${uid}-open)`}>
          <path
            d={paths.inner}
            fill={colors.opening}
            opacity={look.shade_opacity}
            filter={`url(#${uid}-shade)`}
            transform={`translate(${A / 2} ${OPENING_CY}) scale(0.9) translate(${-A / 2} ${-OPENING_CY})`}
          />
        </g>
      ) : null}
      <g clipPath={`url(#${uid}-ring)`}>
        <path d={MARK_RING_PATH} fillRule="evenodd" fill={`url(#${uid}-body)`} />
        <path
          d={paths.outer}
          fill="none"
          stroke={`url(#${uid}-light)`}
          strokeWidth={look.light_width}
          opacity={look.light_opacity}
          filter={detailed ? `url(#${uid}-soft)` : undefined}
        />
        <path
          d={paths.outer}
          fill="none"
          stroke={`url(#${uid}-rim)`}
          strokeWidth={detailed ? look.rim_width : look.rim_width * 2}
          opacity={look.rim_opacity * visual.levels.specular}
          filter={detailed ? `url(#${uid}-fine)` : undefined}
        />
        <path
          d={paths.inner}
          fill="none"
          stroke={`url(#${uid}-edge)`}
          strokeWidth={look.edge_width}
          opacity={look.edge_opacity}
          filter={detailed ? `url(#${uid}-fine)` : undefined}
        />
      </g>
    </svg>
  );
}

function EnergyLayer({ visual, size, angle }: { visual: PresenceVisual; size: number; angle: number }) {
  const { layers, levels } = visual;
  const r = size * ATTENTION_RADIUS;
  const focus = visual.state === "waitingForUser" ? 0.72 : 1;
  const spot = `translate3d(${Math.round(Math.cos(angle) * r * 10) / 10}px, ${
    Math.round(Math.sin(angle) * r * 10) / 10
  }px, 0) scale(${focus})`;
  return (
    <div className="pgm-layer pgm-energy" style={RING_MASK_STYLE}>
      <div className="pgm-layer pgm-charge" style={{ opacity: levels.charge }}>
        <div className={`pgm-fill pgm-charge-fill${gateClass(visual, "charge")}`} style={animationStyle(visual, "charge")} />
      </div>
      {layers.orbit ? (
        <div className={`pgm-orbit${gateClass(visual, "orbit")}`} style={animationStyle(visual, "orbit")} />
      ) : null}
      {layers.arc ? (
        <div className="pgm-layer pgm-arc" style={{ opacity: levels.arc }}>
          <div className={`pgm-fill pgm-arc-fill${gateClass(visual, "arc")}`} style={animationStyle(visual, "arc")} />
        </div>
      ) : null}
      <div className="pgm-attention" style={{ opacity: layers.attention ? levels.attention : 0, transform: spot }} />
      {layers.success ? <div className="pgm-layer pgm-success" style={animationStyle(visual, "success")} /> : null}
      <div className="pgm-layer pgm-dim" style={{ opacity: levels.dim }} />
    </div>
  );
}

function EyesLayer({ uid, visual, offset }: { uid: string; visual: PresenceVisual; offset: Vec2 }) {
  return (
    <svg
      className="pgm-layer pgm-eyes"
      viewBox={`0 0 ${A} ${A}`}
      aria-hidden="true"
      focusable="false"
      style={{
        opacity: visual.levels.eyes,
        transform: offset.x || offset.y ? `translate3d(${offset.x}px, ${offset.y}px, 0)` : undefined,
      }}
    >
      <defs>
        <Gradient id={`${uid}-eye`} stops={colors.eye} y1={EYE_TOP} y2={EYE_BOTTOM} />
        {visual.layers.eyeGlow ? <Blur id={`${uid}-eg`} std={look.eye_glow_std} /> : null}
      </defs>
      {visual.layers.eyeGlow ? (
        <path d={EYES_PATH} fill={palette.electric} opacity={look.eye_glow_opacity} filter={`url(#${uid}-eg)`} />
      ) : null}
      <path d={EYES_PATH} fill={`url(#${uid}-eye)`} />
    </svg>
  );
}

function ErrorBadge() {
  return (
    <span className="pgm-badge" aria-hidden="true">
      <svg viewBox="0 0 20 20" focusable="false">
        <circle cx="10" cy="10" r="9" className="pgm-badge-disc" />
        <rect x="8.75" y="4.5" width="2.5" height="7" rx="1.25" className="pgm-badge-glyph" />
        <circle cx="10" cy="14.6" r="1.45" className="pgm-badge-glyph" />
      </svg>
    </span>
  );
}

/**
 * The Pegoles mark as a living presence. Layers: pre-blurred glow (opacity
 * only), static ring (gradients + rim light + inner shadow), ring-masked
 * energy (charge, traveling light, attention, success, dim), eyes, and a
 * semantic error badge. See `presenceVisual` for the tier / reduced-motion
 * rules; all loops are CSS animations gated by the ambient/offscreen CSS.
 */
export function PegolesMark({
  size = 64,
  state = "idle",
  effectsTier,
  reducedMotion,
  lookAt = null,
  attention = null,
  label = "Pegoles",
  decorative = false,
  className,
  style,
}: PegolesMarkProps) {
  const uid = `pgm${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const effects = useResolvedEffects(effectsTier, reducedMotion);
  const offscreenRef = useOffscreenPause<HTMLDivElement>();
  const visual = presenceVisual({ state, tier: effects.tier, reducedMotion: effects.reducedMotion, size });
  const angle = attentionAngle(attention);
  const shift = visual.glowShift
    ? { x: Math.round(Math.cos(angle) * size * GLOW_LEAN * 10) / 10, y: Math.round(Math.sin(angle) * size * GLOW_LEAN * 10) / 10 }
    : { x: 0, y: 0 };
  const offset = visual.eyeMotion ? eyeOffset(lookAt, size) : { x: 0, y: 0 };
  const a11y = decorative
    ? { "aria-hidden": true as const }
    : { role: "img", "aria-label": `${label} — ${PRESENCE_LABEL[state]}` };

  return (
    <div
      ref={offscreenRef}
      className={className ? `pgm ${className}` : "pgm"}
      data-presence={state}
      data-tier={visual.tier}
      data-rm={visual.reducedMotion ? "true" : "false"}
      data-compact={visual.compact ? "true" : "false"}
      style={
        {
          ...style,
          width: size,
          height: size,
          "--pgm-fade": `${visual.fadeMs}ms`,
          "--pgm-breathe-min": String(visual.breatheMin),
        } as CSSProperties
      }
      {...a11y}
    >
      {visual.layers.glow ? <GlowLayer uid={uid} visual={visual} shift={shift} /> : null}
      <RingLayer uid={uid} visual={visual} />
      {visual.layers.energy ? <EnergyLayer visual={visual} size={size} angle={angle} /> : null}
      <EyesLayer uid={uid} visual={visual} offset={offset} />
      {visual.layers.badge ? <ErrorBadge /> : null}
    </div>
  );
}

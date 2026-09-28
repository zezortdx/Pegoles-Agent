import { memo } from "react";
import { MARK_ARTBOARD, MARK_GEOMETRY } from "@pegoles/ui";
import bodyLarge from "./art/body-512.webp";
import bodySmall from "./art/body-128.webp";
import eyeLeft from "./art/eye-left.webp";
import eyeRight from "./art/eye-right.webp";

const { outer: OUTER, inner: INNER } = MARK_GEOMETRY.paths;
const C = MARK_ARTBOARD / 2;
const around = (scale: number) => `translate(${C} ${C}) scale(${scale}) translate(${-C} ${-C})`;
/** Largest mark (CSS px) that the 128 px body still covers at 2x. */
const SMALL_ART_MAX = 40;
const EYE_ART = [eyeLeft, eyeRight] as const;

/** Eye boxes as fractions of the artboard, from the fitted geometry (left eye first). */
export const EYE_BOXES = [...MARK_GEOMETRY.eyes].sort((a, b) => a.cx - b.cx).map((eye) => ({
  left: (eye.cx - eye.width / 2) / MARK_ARTBOARD,
  top: (eye.cy - eye.height / 2) / MARK_ARTBOARD,
  width: eye.width / MARK_ARTBOARD,
  height: eye.height / MARK_ARTBOARD,
}));

/**
 * The official mark's own pixels (scripts/brand/presence-art.py): ring,
 * face and glow, with the eyes lifted out so they can blink and glance.
 * The art spans the artboard plus a quarter on every side for the glow.
 */
const Body = memo(function Body({ small }: { small: boolean }) {
  return <img className="presence__art" src={small ? bodySmall : bodyLarge} alt="" draggable={false} decoding="async" />;
});

/** An outline of the mark, drawn with a hairline at any scale. */
function Outline({ className, scale = 1, d = OUTER }: { className: string; scale?: number; d?: string }) {
  return <svg className={className} viewBox={`0 0 ${MARK_ARTBOARD} ${MARK_ARTBOARD}`} aria-hidden="true" focusable="false">
    <path d={d} transform={scale === 1 ? undefined : around(scale)} vectorEffect="non-scaling-stroke" />
  </svg>;
}

const Filaments = memo(function Filaments() {
  return <svg className="presence__filaments" viewBox={`0 0 ${MARK_ARTBOARD} ${MARK_ARTBOARD}`} aria-hidden="true" focusable="false">
    <path d={OUTER} transform={around(0.955)} vectorEffect="non-scaling-stroke" />
    <path d={OUTER} transform={around(0.91)} vectorEffect="non-scaling-stroke" />
    <path d={INNER} transform={around(1.05)} vectorEffect="non-scaling-stroke" />
  </svg>;
});

export interface PresenceArtProps {
  /** Rendered size in CSS px: picks the body resolution. */
  readonly size: number;
  readonly pulse: number;
  readonly field: boolean;
  /** Continuous-motion class: decorative life or real work (see the ambient gate). */
  readonly loopClass: string;
}

/**
 * The only renderer of Pegoles. The mark is the official artwork; state
 * lives in light around it (halo, field contours, working sweep, amber
 * rim) and in the eyes. Every animated layer is an HTML element moved by
 * transform and opacity; nothing inside it re-rasterizes.
 */
export function PresenceArt({ size, pulse, field, loopClass }: PresenceArtProps) {
  return <div className="presence__stage" aria-hidden="true">
    <div className="presence__halo" />
    <div className="presence__flash" />
    {field && <div className="presence__field">
      <Outline className={`presence__contour ${loopClass}`} />
      <Outline className={`presence__contour ${loopClass}`} />
      <Outline className={`presence__contour ${loopClass}`} />
      <Outline className="presence__shimmer" />
      {pulse > 0 && <Outline key={pulse} className="presence__pulse" />}
    </div>}
    <div className="presence__tilt">
      <div className="presence__react">
        <div className={`presence__object ${loopClass}`}>
          <Outline className="presence__shell-layer" scale={1.035} />
          <Body small={size <= SMALL_ART_MAX} />
          <div className="presence__sweep"><div className={`presence__sweep-band ${loopClass}`} /></div>
          <Filaments />
          <Outline className="presence__warn-rim" />
          <Outline className="presence__bloom" />
          {EYE_BOXES.map((box, index) => <div key={index} className="presence__eye" style={{
            left: `${box.left * 100}%`, top: `${box.top * 100}%`, width: `${box.width * 100}%`, height: `${box.height * 100}%`,
          }}><div className="presence__eye-lid" style={{ backgroundImage: `url(${EYE_ART[index]})` }} /></div>)}
        </div>
      </div>
    </div>
  </div>;
}

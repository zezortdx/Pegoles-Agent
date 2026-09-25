import { memo, useId } from "react";
import { MARK_ARTBOARD, MARK_GEOMETRY } from "@pegoles/ui";

const { outer: OUTER, inner: INNER } = MARK_GEOMETRY.paths;
const RING = `${OUTER}${INNER}`;
const C = MARK_ARTBOARD / 2;
const around = (scale: number) => `translate(${C} ${C}) scale(${scale}) translate(${-C} ${-C})`;

/** Eye boxes as fractions of the artboard, from the fitted geometry. */
export const EYE_BOXES = MARK_GEOMETRY.eyes.map((eye) => ({
  left: (eye.cx - eye.width / 2) / MARK_ARTBOARD,
  top: (eye.cy - eye.height / 2) / MARK_ARTBOARD,
  width: eye.width / MARK_ARTBOARD,
  height: eye.height / MARK_ARTBOARD,
}));

/** Static body: frosted white ceramic shell and a black glass face. Rasterized once per size. */
const Body = memo(function Body() {
  const id = useId().replaceAll(":", "");
  const ref = (name: string) => `url(#${id}-${name})`;
  return <svg className="presence__body" viewBox={`0 0 ${MARK_ARTBOARD} ${MARK_ARTBOARD}`} aria-hidden="true" focusable="false">
    <defs>
      <linearGradient id={`${id}-shell`} x1="0.14" y1="0.02" x2="0.86" y2="1">
        <stop offset="0" stopColor="#ffffff" />
        <stop offset="0.34" stopColor="#f2f3f4" />
        <stop offset="0.7" stopColor="#d3d6da" />
        <stop offset="1" stopColor="#92979d" />
      </linearGradient>
      <radialGradient id={`${id}-key`} cx="0.27" cy="0.14" r="0.82">
        <stop offset="0" stopColor="#ffffff" stopOpacity="0.95" />
        <stop offset="0.42" stopColor="#ffffff" stopOpacity="0.22" />
        <stop offset="1" stopColor="#ffffff" stopOpacity="0" />
      </radialGradient>
      <linearGradient id={`${id}-rim`} x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor="#ffffff" stopOpacity="1" />
        <stop offset="0.5" stopColor="#ffffff" stopOpacity="0.6" />
        <stop offset="1" stopColor="#dfe8f5" stopOpacity="0.28" />
      </linearGradient>
      <linearGradient id={`${id}-face`} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stopColor="#141518" />
        <stop offset="0.28" stopColor="#050506" />
        <stop offset="1" stopColor="#000000" />
      </linearGradient>
      <linearGradient id={`${id}-lip`} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0.6" stopColor="#ffffff" stopOpacity="0" />
        <stop offset="1" stopColor="#ffffff" stopOpacity="0.5" />
      </linearGradient>
      <radialGradient id={`${id}-visor`} cx="0.5" cy="0" r="0.75">
        <stop offset="0" stopColor="#ffffff" stopOpacity="0.085" />
        <stop offset="1" stopColor="#ffffff" stopOpacity="0" />
      </radialGradient>
      <clipPath id={`${id}-ring`}><path d={RING} clipRule="evenodd" /></clipPath>
      <clipPath id={`${id}-face-clip`}><path d={INNER} /></clipPath>
      <filter id={`${id}-soft`} x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="9" /></filter>
    </defs>
    <path d={RING} fillRule="evenodd" fill={ref("shell")} />
    <path d={RING} fillRule="evenodd" fill={ref("key")} />
    <g clipPath={ref("ring")}>
      {/* Light passing through the thin outer edge: the frosted, translucent band. */}
      <path d={OUTER} fill="none" stroke="#ffffff" strokeOpacity="0.55" strokeWidth="30" filter={ref("soft")} />
      {/* The shell curving down into the face. */}
      <path d={INNER} fill="none" stroke="#000000" strokeOpacity="0.42" strokeWidth="36" filter={ref("soft")} />
    </g>
    <path d={OUTER} fill="none" stroke={ref("rim")} strokeWidth="3" />
    <path d={INNER} fill={ref("face")} />
    <g clipPath={ref("face-clip")}>
      <ellipse cx={C} cy="112" rx="300" ry="140" fill={ref("visor")} />
    </g>
    <path d={INNER} fill="none" stroke={ref("lip")} strokeWidth="2.5" />
  </svg>;
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

export interface PresenceSvgProps {
  readonly pulse: number;
  readonly field: boolean;
  /** Continuous-motion class: decorative life or real work (see the ambient gate). */
  readonly loopClass: string;
}

/**
 * First paint, fallback and small-size renderer. Every animated layer is an
 * HTML element (compositor transforms and opacity); the SVGs inside never
 * change, so WebKit rasterizes them once.
 */
export function PresenceSvg({ pulse, field, loopClass }: PresenceSvgProps) {
  return <div className="presence__svg" aria-hidden="true">
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
          <Body />
          {/* A soft highlight drifting across the glaze: the SVG echo of the studio light. */}
          <div className="presence__sheen"><div className={`presence__sheen-light ${loopClass}`} /></div>
          <div className="presence__sweep"><div className={`presence__sweep-band ${loopClass}`} /></div>
          <Filaments />
          <Outline className="presence__warn-rim" />
          <Outline className="presence__bloom" />
          {EYE_BOXES.map((box, index) => <div key={index} className="presence__eye" style={{
            left: `${box.left * 100}%`, top: `${box.top * 100}%`, width: `${box.width * 100}%`, height: `${box.height * 100}%`,
          }}><div className="presence__eye-lid" /></div>)}
        </div>
      </div>
    </div>
  </div>;
}

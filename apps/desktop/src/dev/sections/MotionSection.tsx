import { useRef, useState } from "react";
import { animate } from "motion";
import {
  duration,
  easing,
  easingBezier,
  GlassButton,
  PlayIcon,
  spring,
  useFluxGlass,
  type DurationToken,
  type SpringToken,
} from "@pegoles/ui";
import { LabSection } from "../lab/controls";

const TIMED: readonly { token: DurationToken; use: string }[] = [
  { token: "instant", use: "Press feedback" },
  { token: "interaction", use: "Hover, focus light, crossfades" },
  { token: "standard", use: "Small state changes" },
  { token: "fluid", use: "Energy edge, surface state" },
  { token: "scene", use: "Scene changes (rare)" },
];

function CurvePlot({ points }: { readonly points: readonly [number, number, number, number] }) {
  const [x1, y1, x2, y2] = points;
  const s = 56;
  const d = `M0 ${s} C ${x1 * s} ${s - y1 * s}, ${x2 * s} ${s - y2 * s}, ${s} 0`;
  return (
    <svg className="lab-curve" viewBox={`-4 -4 ${s + 8} ${s + 8}`} aria-hidden="true">
      <rect x="0" y="0" width={s} height={s} className="lab-curve__frame" />
      <path d={d} className="lab-curve__path" />
    </svg>
  );
}

export function MotionSection() {
  const { reducedMotion } = useFluxGlass();
  const [played, setPlayed] = useState(false);
  const springRefs = useRef<Partial<Record<SpringToken, HTMLSpanElement | null>>>({});
  const springOut = useRef(false);

  const playSprings = () => {
    springOut.current = !springOut.current;
    for (const key of Object.keys(spring) as SpringToken[]) {
      const el = springRefs.current[key];
      const track = el?.parentElement;
      if (!el || !track) continue;
      const travel = track.clientWidth - el.clientWidth;
      if (reducedMotion) {
        animate(el, { opacity: [0.3, 1] }, { duration: duration.interaction / 1000 });
        continue;
      }
      animate(el, { transform: `translateX(${springOut.current ? travel : 0}px)` }, spring[key]);
    }
  };

  return (
    <LabSection
      id="lab-motion"
      index="09"
      title="Motion"
      lead="Motion explains a change or it does not happen. Keyboard-summoned surfaces appear instantly; state changes ease out; spatial moves use critically damped springs. Under reduced motion, movement becomes light and opacity."
    >
      <div className="lab-motion">
        <div className="lab-motion__block">
          <div className="lab-motion__head">
            <h3 className="lab-subhead">Durations · ease-out</h3>
            <GlassButton size="sm" icon={<PlayIcon size={12} />} onClick={() => setPlayed((p) => !p)}>
              Play
            </GlassButton>
          </div>
          <ul className="lab-tracks" data-played={played ? "true" : "false"}>
            {TIMED.map(({ token, use }) => (
              <li key={token} className="lab-track">
                <span className="lab-track__name">{token}</span>
                <span className="lab-mono">{duration[token]} ms</span>
                <span className="lab-track__rail">
                  <span className="lab-track__runner" style={{ transitionDuration: `${duration[token]}ms` }}>
                    <span className="lab-track__puck" />
                  </span>
                </span>
                <span className="lab-track__use">{use}</span>
              </li>
            ))}
          </ul>
          <p className="lab-footnote">
            Ambient loops live in a {duration.ambientMin / 1000}–{duration.ambientMax / 1000} s window and sleep with the
            idle gate.
          </p>
        </div>

        <div className="lab-motion__block">
          <h3 className="lab-subhead">Curves</h3>
          <ul className="lab-curves">
            {(Object.keys(easingBezier) as (keyof typeof easingBezier)[]).map((key) => (
              <li key={key}>
                <CurvePlot points={easingBezier[key]} />
                <span className="lab-track__name">{key}</span>
                <span className="lab-mono lab-curves__value">{easing[key].replace("cubic-bezier", "")}</span>
              </li>
            ))}
          </ul>
        </div>

        <div className="lab-motion__block">
          <div className="lab-motion__head">
            <h3 className="lab-subhead">Springs</h3>
            <GlassButton size="sm" icon={<PlayIcon size={12} />} onClick={playSprings}>
              Play
            </GlassButton>
          </div>
          <ul className="lab-tracks">
            {(Object.keys(spring) as SpringToken[]).map((key) => (
              <li key={key} className="lab-track">
                <span className="lab-track__name">{key}</span>
                <span className="lab-mono">
                  {spring[key].visualDuration}s · bounce {spring[key].bounce}
                </span>
                <span className="lab-track__rail">
                  <span
                    className="lab-track__puck lab-track__puck--spring"
                    ref={(node) => {
                      springRefs.current[key] = node;
                    }}
                  />
                </span>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </LabSection>
  );
}

import { useId, useLayoutEffect, useRef } from "react";
import { PresenceMark } from "../presence/PresenceMark";
import type { PresenceMode } from "../presence/modes";
import { isInFlight, type RibbonModel, type RibbonPhase, type RibbonStrand } from "../state/ribbon";
import "./thoughtRibbon.css";

export interface ThoughtRibbonProps {
  readonly model: RibbonModel;
  /** Mini presence size in CSS px. */
  readonly size?: number;
  readonly reducedMotion?: boolean;
  /** Strands grow from a presence drawn by the host (e.g. the task's turn): no mark of its own. */
  readonly anchored?: boolean;
  readonly className?: string;
}

/** Layout (CSS px). The whole ribbon stays within ≈ 220 × 72. */
const WIDTH = 220;
const MAX_HEIGHT = 72;
const ROW = 16;
const GAP = 3;
const STUB = 6;
const CURVE = 20;
const RUN = 20;
/** Motion (ms), mirroring the motion tokens: sprout is a state change, the rest are micro. */
const SPROUT_MS = 420;
const RETRACT_MS = 360;
const SHIFT_MS = 240;
const FADE_MS = 160;
const EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";

const SR_PHASE: Record<RibbonPhase, string> = {
  sprouting: "starting",
  approval: "needs your approval",
  running: "in progress",
  done: "done",
  failed: "didn’t work",
  blocked: "stopped by a safety rule",
  interrupted: "interrupted",
};

function presenceMode(model: RibbonModel, active: RibbonStrand | undefined): PresenceMode {
  switch (model.trunk) {
    case "dormant": return "idle";
    case "thinking": return "thinking";
    case "working": return active?.capability === "Computer" ? "using-computer" : "working";
    case "waiting": return active?.phase === "approval" ? "needs-user" : "waiting";
    case "done": return "done";
  }
}

interface Row { readonly strand: RibbonStrand; readonly y: number; readonly d: string }

function layout(model: RibbonModel, size: number, minHeight = size) {
  const n = model.strands.length;
  const height = Math.min(MAX_HEIGHT, Math.max(minHeight, n * ROW) + 8);
  const cy = height / 2;
  const x0 = size + GAP;
  const x1 = x0 + STUB + CURVE + RUN;
  const rows: Row[] = model.strands.map((strand, i) => {
    const y = cy + (i - (n - 1) / 2) * ROW;
    const xs = x0 + STUB;
    return { strand, y, d: `M ${x0} ${cy} L ${xs} ${cy} C ${xs + 10} ${cy}, ${xs + CURVE - 10} ${y}, ${xs + CURVE} ${y} L ${x1} ${y}` };
  });
  return { height, cy, x0, x1, rows };
}

function animate(el: Element | null | undefined, keyframes: Keyframe[], options: KeyframeAnimationOptions): void {
  if (el && typeof (el as HTMLElement).animate === "function") (el as HTMLElement).animate(keyframes, options);
}

/**
 * The reasoning visual, honest: a mini Presence with thin strands of light
 * growing to its right, one per real action (see state/ribbon.ts). Only
 * the active strand is luminous; finished ones settle to a faint trace.
 * SVG + Web Animations one-shots; the only loop is the running signal,
 * a CSS animation under the pg-work-anim gate. No rAF, no WebGL, no filter
 * animation (the glow is a pre-blurred duplicate path).
 */
export function ThoughtRibbon({ model, size = 36, reducedMotion = false, anchored = false, className }: ThoughtRibbonProps) {
  const root = useRef<HTMLDivElement>(null);
  const glowId = `thought-ribbon-glow-${useId().replaceAll(":", "")}`;
  const seen = useRef<Map<string, { phase: RibbonPhase; y: number }> | null>(null);
  const { height, cy, x0, x1, rows } = layout(model, anchored ? 0 : size, anchored ? size : undefined);
  const active = model.strands.find((strand) => strand.id === model.activeId);

  useLayoutEffect(() => {
    const host = root.current;
    const first = seen.current === null;
    const previous = seen.current ?? new Map<string, { phase: RibbonPhase; y: number }>();
    const next = new Map<string, { phase: RibbonPhase; y: number }>();
    for (const { strand, y } of rows) {
      next.set(strand.id, { phase: strand.phase, y });
      if (first || !host) continue;
      const q = (part: string) => host.querySelector(`[data-strand="${CSS.escape(strand.id)}"]${part}`);
      const was = previous.get(strand.id);
      if (!was) {
        // A real action was requested: its strand sprouts from the trunk.
        if (reducedMotion) {
          animate(q(".thought-ribbon__strand"), [{ opacity: 0 }, { opacity: 1 }], { duration: FADE_MS, easing: "linear" });
        } else {
          animate(q(" .thought-ribbon__base"), [{ strokeDashoffset: 100 }, { strokeDashoffset: 0 }], { duration: SPROUT_MS, easing: EASE_OUT });
          animate(q(" .thought-ribbon__lit"), [{ strokeDashoffset: 100 }, { strokeDashoffset: 0 }], { duration: SPROUT_MS, easing: EASE_OUT });
        }
        animate(q(".thought-ribbon__label"), [{ opacity: 0 }, { opacity: 1 }], { duration: reducedMotion ? FADE_MS : SPROUT_MS, easing: EASE_OUT, delay: reducedMotion ? 0 : 140, fill: "backwards" });
        continue;
      }
      if (isInFlight(was.phase) && !isInFlight(strand.phase)) {
        // Finished: its light pulls back into the trunk; the trace stays faint.
        const lit = q(" .thought-ribbon__lit-ghost");
        if (reducedMotion) animate(lit, [{ opacity: 1 }, { opacity: 0 }], { duration: FADE_MS, easing: "linear" });
        else animate(lit, [{ strokeDashoffset: 0, opacity: 1 }, { strokeDashoffset: 100, opacity: 0.6 }], { duration: RETRACT_MS, easing: EASE_OUT });
      }
      if (was.y !== y && !reducedMotion) {
        const shift = [{ transform: `translateY(${was.y - y}px)` }, { transform: "translateY(0)" }];
        animate(q(".thought-ribbon__strand"), shift, { duration: SHIFT_MS, easing: EASE_OUT });
        animate(q(".thought-ribbon__label"), shift, { duration: SHIFT_MS, easing: EASE_OUT });
      }
    }
    seen.current = next;
  });

  const trunk = model.trunk;
  return <div ref={root} className={className ? `thought-ribbon ${className}` : "thought-ribbon"} data-trunk={trunk}
    style={{ width: WIDTH, height, ["--ribbon-mark" as string]: `${size}px` }}>
    {!anchored && <PresenceMark mode={presenceMode(model, active)} size={size} className="thought-ribbon__mark" />}
    <svg className="thought-ribbon__svg" width={WIDTH} height={height} viewBox={`0 0 ${WIDTH} ${height}`} aria-hidden="true" focusable="false">
      <defs>
        <filter id={glowId} x="-10%" y="-200%" width="120%" height="500%"><feGaussianBlur stdDeviation="1.6" /></filter>
      </defs>
      <path className="thought-ribbon__trunk" d={`M ${x0} ${cy} L ${x0 + STUB} ${cy}`} />
      {rows.map(({ strand, d, y }) => {
        const lit = strand.id === model.activeId;
        const cut = strand.phase === "failed" || strand.phase === "blocked" || strand.phase === "interrupted";
        return <g key={strand.id} className="thought-ribbon__strand" data-strand={strand.id} data-phase={strand.phase} data-active={lit || undefined}>
          <path className="thought-ribbon__base" d={d} pathLength={100} />
          {lit && <path className="thought-ribbon__glow" d={d} pathLength={100} filter={`url(#${glowId})`} />}
          {lit && <path className="thought-ribbon__lit" d={d} pathLength={100} />}
          {/* Kept mounted after the action finishes so the light can retract. */}
          <path className="thought-ribbon__lit-ghost" d={d} pathLength={100} />
          {strand.phase === "running" && lit && !reducedMotion && <path className="thought-ribbon__signal pg-work-anim" d={d} pathLength={100} />}
          {strand.phase === "approval" && <circle className="thought-ribbon__node" cx={x1} cy={y} r={2.5} />}
          {cut && strand.phase !== "interrupted" && <path className="thought-ribbon__cap" d={`M ${x1 - 1} ${y - 3.5} L ${x1 - 1} ${y + 3.5}`} />}
        </g>;
      })}
    </svg>
    <ul className="thought-ribbon__labels" aria-label="What Pegoles is doing">
      {rows.map(({ strand, y }) => <li key={strand.id} className="thought-ribbon__label" data-strand={strand.id}
        data-active={strand.id === model.activeId || undefined} data-phase={strand.phase}
        style={{ top: y - 8, left: x1 + 6 }}>
        {strand.label}<span className="thought-ribbon__sr">, {SR_PHASE[strand.phase]}</span>
      </li>)}
    </ul>
  </div>;
}

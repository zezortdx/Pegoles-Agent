import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { observeOffscreen, useAmbientState, useFluxGlass } from "@pegoles/ui";
import { useSystemReducedMotion } from "../lib/useSystemReducedMotion";
import { subscribeAvatarClock } from "./animationClock";
import { AvatarController, engineStateForMode, MIN_ORBIT_SIZE, staticFrameForMode } from "./controller";
import type { BotFrame } from "./engine/engine";
import { createThrottle } from "./life";
import { PRESENCE_LABEL, WORK_MODES, type PresenceMode } from "./modes";
import "./presence.css";

export interface PresenceAvatarProps {
  readonly mode: PresenceMode;
  readonly size: number;
  readonly look?: { readonly x: number; readonly y: number } | null;
  readonly interactive?: boolean;
  /** Counters change only on actual user actions or Core events. */
  readonly pulse?: number;
  readonly nudge?: number;
  readonly reaction?: number;
  readonly field?: boolean;
  readonly decorative?: boolean;
  readonly label?: string;
  readonly className?: string;
}

interface PointerPose { eyeX: number; eyeY: number; pitch: number; yaw: number }
const TRACK_X = 0.08;
const TRACK_Y = 0.05;

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(Math.max((x - edge0) / (edge1 - edge0), 0), 1);
  return t * t * (3 - 2 * t);
}

/** Pointer response eases with distance and stays within the existing gaze bounds. */
export function pointerPose(dx: number, dy: number): PointerPose {
  const distance = Math.hypot(dx, dy);
  if (distance < 1e-3) return { eyeX: 0, eyeY: 0, pitch: 0, yaw: 0 };
  const reach = Math.min(distance / 260, 1);
  const ux = (dx / distance) * reach;
  const uy = (dy / distance) * reach;
  const track = 1 - 0.65 * smoothstep(260, 900, distance);
  const lean = 1 - smoothstep(0, 560, distance);
  const tilt = lean * lean * (3 - 2 * lean);
  return { eyeX: ux * TRACK_X * track, eyeY: uy * TRACK_Y * track, pitch: uy * 7 * tilt, yaw: ux * 7 * tilt };
}

/** SVG geometry is generated locally from typed poses, never from guest/model markup. */
export function PresenceAvatar({ mode, size, look = null, interactive = false, pulse = 0, nudge = 0, reaction = 0,
  field = size >= 40, decorative = false, label, className }: PresenceAvatarProps) {
  const { reducedMotion, tier } = useFluxGlass();
  const systemReducedMotion = useSystemReducedMotion();
  const ambient = useAmbientState();
  const staticPose = reducedMotion || systemReducedMotion || tier === "minimal" || ["offline", "blocked", "error"].includes(mode);
  const [controller] = useState(() => new AvatarController(mode, size));
  const presentationKey = `${mode}:${size < MIN_ORBIT_SIZE}:${staticPose}`;
  const fallbackFrame = useMemo(() => staticFrameForMode(mode, size), [mode, size]);
  const [sample, setSample] = useState(() => ({
    key: presentationKey, frame: staticPose ? fallbackFrame : controller.sample(0), engineState: controller.engineState,
  }));
  const updateFrame = useCallback((frame: BotFrame) => {
    setSample({ key: presentationKey, frame, engineState: controller.engineState });
  }, [controller, presentationKey]);
  // Props may change before effects run. Never paint an old orbit under a new state label.
  const matching = sample.key === presentationKey;
  const frame = matching ? sample.frame : fallbackFrame;
  const engineState = staticPose || !matching ? engineStateForMode(mode, size) : sample.engineState;
  const [offscreen, setOffscreen] = useState(false);
  const [pulseBase] = useState(pulse);
  const root = useRef<SVGSVGElement>(null);
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const latestLook = useRef(look);
  latestLook.current = look;
  const previousMode = useRef(mode);
  const previousNudge = useRef(nudge);
  const previousReaction = useRef(reaction);
  const nudgeGate = useRef(createThrottle(90));
  const uid = `presence-avatar-${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
  const working = WORK_MODES.has(mode);
  const running = !staticPose && !offscreen && (!ambient || (working ? ambient.visible : ambient.running));

  useEffect(() => {
    controller.setMode(mode, size);
    if (staticPose) controller.settle();
    else if (previousMode.current !== mode && (mode === "acknowledging" || mode === "done")) controller.react("acknowledge");
    previousMode.current = mode;
    updateFrame(staticPose ? controller.staticFrame() : controller.sample(0));
  }, [controller, mode, size, staticPose, updateFrame]);

  useEffect(() => {
    const changed = nudge !== previousNudge.current;
    previousNudge.current = nudge;
    if (changed && !staticPose && nudgeGate.current()) {
      controller.react("nudge");
      updateFrame(controller.sample(0));
    }
  }, [controller, nudge, staticPose, updateFrame]);

  useEffect(() => {
    const changed = reaction !== previousReaction.current;
    previousReaction.current = reaction;
    if (changed && !staticPose) {
      controller.react("acknowledge");
      updateFrame(controller.sample(0));
    }
  }, [controller, reaction, staticPose, updateFrame]);

  useEffect(() => root.current ? observeOffscreen(root.current, setOffscreen) : undefined, []);

  useEffect(() => {
    if (!interactive || staticPose) return;
    const move = (event: PointerEvent) => {
      if (event.pointerType !== "touch") pointer.current = { x: event.clientX, y: event.clientY };
    };
    const leave = () => { pointer.current = null; };
    window.addEventListener("pointermove", move, { passive: true });
    document.documentElement.addEventListener("pointerleave", leave);
    return () => {
      window.removeEventListener("pointermove", move);
      document.documentElement.removeEventListener("pointerleave", leave);
      pointer.current = null;
    };
  }, [interactive, staticPose]);

  useEffect(() => {
    if (!running) return;
    let elapsed = 0;
    const interval = working && size >= MIN_ORBIT_SIZE ? 1 / 60 : 1 / 30;
    return subscribeAvatarClock((dt) => {
      elapsed += dt;
      if (elapsed < interval) return;
      const at = interactive ? pointer.current : null;
      const box = at ? root.current?.getBoundingClientRect() : null;
      if (at && box && box.width > 0 && box.height > 0) {
        const pose = pointerPose(at.x - (box.left + box.width / 2), at.y - (box.top + box.height / 2));
        controller.setLook({ x: pose.eyeX / TRACK_X, y: pose.eyeY / TRACK_Y });
      } else controller.setLook(latestLook.current);
      updateFrame(controller.sample(elapsed));
      elapsed = 0;
    });
  }, [controller, interactive, running, size, working, updateFrame]);

  const bodyFill = `url(#${uid}-silver)`;
  const arcs = size >= MIN_ORBIT_SIZE ? frame.arcs : [];
  const dots = (list: BotFrame["dots"]) => list.map((dot, index) => {
    const fill = dot.depth === undefined ? bodyFill
      : `color-mix(in srgb, var(--presence-silver, #a7abb0) ${Math.round(dot.depth * 100)}%, var(--window, #111214))`;
    return dot.d
      ? <path key={index} d={dot.d} transform={`translate(${dot.x} ${dot.y}) rotate(${dot.rot ?? 0}) scale(100)`} fill={fill} opacity={dot.opacity} />
      : <circle key={index} cx={dot.x} cy={dot.y} r={dot.r} fill={fill} opacity={dot.opacity} />;
  });
  const a11y = decorative ? { "aria-hidden": true as const } : { role: "img", "aria-label": label ?? PRESENCE_LABEL[mode] };

  return <svg ref={root} className={className ? `presence-avatar ${className}` : "presence-avatar"}
    width={size} height={size} viewBox="-112 -112 224 224" focusable="false" data-mode={mode}
    data-frame={staticPose ? "static" : "live"} data-engine-state={engineState} {...a11y}>
    <defs>
      <linearGradient id={`${uid}-silver`} x1="0" y1="0" x2="0.8" y2="1">
        <stop offset="0" stopColor="var(--presence-white, #f5f5f3)" />
        <stop offset="0.48" stopColor="var(--presence-silver, #a7abb0)" />
        <stop offset="1" stopColor="var(--presence-white, #f5f5f3)" />
      </linearGradient>
      <clipPath id={`${uid}-body`}><path d={frame.bodyPath} /></clipPath>
    </defs>
    {field && <circle className="presence-avatar__halo" cx="0" cy="0" r="100" />}
    {field && !staticPose && pulse !== pulseBase && <circle key={pulse} className="presence-avatar__pulse pg-work-anim" cx="0" cy="0" r="100" />}
    <g fill="none" strokeLinecap="round">
      {arcs.map((arc) => <path className="presence-avatar__orbit" key={arc.id} d={arc.back} stroke={bodyFill} strokeWidth={arc.width} opacity={arc.opacity} />)}
    </g>
    {frame.dotsBehind && <g>{dots(frame.dots)}</g>}
    <g opacity={frame.bodyAlpha}>
      <path className="presence-avatar__body" d={frame.bodyPath} fill={bodyFill} />
      <g clipPath={`url(#${uid}-body)`} fill="var(--presence-face, #000000)">
        {frame.eyes.map((eye, index) => <path className="presence-avatar__eye" key={index} d={eye.d} transform={eye.matrix} opacity={eye.alpha} />)}
      </g>
    </g>
    {!frame.dotsBehind && <g>{dots(frame.dots)}</g>}
    {frame.notification && <circle className="presence-avatar__notification" cx={frame.notification.x} cy={frame.notification.y} r={frame.notification.r} fill="var(--warn, #f2b14c)" />}
    <g fill="none" strokeLinecap="round">
      {arcs.map((arc) => <path className="presence-avatar__orbit" key={arc.id} d={arc.front} stroke={bodyFill} strokeWidth={arc.width} opacity={arc.opacity} />)}
    </g>
  </svg>;
}

import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { MARK_RING_MASK, observeOffscreen, useFluxGlass } from "@pegoles/ui";
import { createThrottle, LifeScheduler, Timers } from "./life";
import { GESTURE_MODES, PRESENCE_LABEL, WORK_MODES, type PresenceMode } from "./modes";
import { PresenceArt } from "./PresenceArt";
import * as motion from "./motion";
import type { PointerPose } from "./motion";
import { poseVariables, presenceTargets } from "./targets";
import "./presence.css";

export interface PegolesPresenceProps {
  mode: PresenceMode;
  /** CSS px of the mark's bounding box (hero ~152, transcript ~40, sidebar 18). */
  size: number;
  /** Normalized gaze direction (−1..1), e.g. toward the composer {0, 1}. */
  look?: { x: number; y: number } | null;
  /** Increments on each real event: one ripple through the thought contours. */
  pulse?: number;
  /** Increments on each keystroke: a tiny reaction (throttled to one per 90 ms). */
  nudge?: number;
  /** Hero only: the eyes follow the pointer and the body tilts toward it. */
  interactive?: boolean;
  /** Hero only: a real button; pressing it gets a boop. */
  pressable?: boolean;
  /** Called after a press, in addition to the boop. */
  onPress?: () => void;
  /** Allow thought contours beyond the mark. Default: size ≥ 40. */
  field?: boolean;
  /** aria-hidden when true; otherwise role="img" with a state label. */
  decorative?: boolean;
  /** Overrides the default sentence for the mode. */
  label?: string;
  className?: string;
}

const NUDGE_INTERVAL_MS = 90;
/** Pointer response (CSS px from the presence centre). */
const NEAR_PX = 260;
const FAR_PX = 900;
const TILT_RADIUS_PX = 560;
const AWARE_IN_PX = 220;
const AWARE_OUT_PX = 250;
/** Eye travel toward the pointer, as a fraction of the mark size. */
const TRACK_X = 0.08;
const TRACK_Y = 0.05;
const TILT_DEG = 7;

/** Pointer position → eye offset and tilt, eased by distance. Pure. */
export function pointerPose(dx: number, dy: number): PointerPose {
  const distance = Math.hypot(dx, dy);
  if (distance < 1e-3) return { eyeX: 0, eyeY: 0, pitch: 0, yaw: 0 };
  const reach = Math.min(distance / NEAR_PX, 1);
  const ux = (dx / distance) * reach;
  const uy = (dy / distance) * reach;
  // Full attention nearby, still noticing across the window.
  const track = 1 - 0.65 * smoothstep(NEAR_PX, FAR_PX, distance);
  const lean = 1 - smoothstep(0, TILT_RADIUS_PX, distance);
  const tilt = lean * lean * (3 - 2 * lean);
  return { eyeX: ux * TRACK_X * track, eyeY: uy * TRACK_Y * track, pitch: uy * TILT_DEG * tilt, yaw: ux * TILT_DEG * tilt };
}

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(Math.max((x - edge0) / (edge1 - edge0), 0), 1);
  return t * t * (3 - 2 * t);
}

/**
 * The single visual representation of Pegoles: the official mark
 * (PresenceArt) with its state expressed in light and in the eyes. Idle
 * life, pointer tracking and reactions write CSS variables directly —
 * none of them re-render React.
 */
export function PegolesPresence({ mode, size, look = null, pulse = 0, nudge = 0, interactive = false, pressable = false, onPress, field, decorative = false, label, className }: PegolesPresenceProps) {
  const { reducedMotion, ambient } = useFluxGlass();
  const showField = field ?? size >= 40;
  const rootRef = useRef<HTMLDivElement>(null);
  const timers = useRef<Timers | null>(null);
  timers.current ??= new Timers();
  // Ripples and nudges answer what happens while mounted, not the history before.
  const [pulseBase] = useState(pulse);
  const [nudgeBase] = useState(nudge);
  const ripple = pulse === pulseBase ? 0 : pulse;
  const keystroke = nudge === nudgeBase ? 0 : nudge;
  const nudgeGate = useRef(createThrottle(NUDGE_INTERVAL_MS));
  const targets = presenceTargets(mode, { reducedMotion });
  const lively = targets.life;
  const latestLook = useRef(look);
  latestLook.current = look;
  const reducedRef = useRef(reducedMotion);
  reducedRef.current = reducedMotion;

  useEffect(() => () => timers.current?.clear(), []);

  // One-shot gestures answer a change of mode.
  const previousMode = useRef(mode);
  useEffect(() => {
    const root = rootRef.current;
    if (!root || previousMode.current === mode) return;
    previousMode.current = mode;
    motion.gesture(root, mode, reducedMotion);
  }, [mode, reducedMotion]);

  // Keystrokes: widen, lean, shimmer — at most one per 90 ms.
  useEffect(() => {
    const root = rootRef.current;
    if (!root || keystroke === 0 || !nudgeGate.current()) return;
    motion.nudge(root, latestLook.current, timers.current ?? new Timers(), reducedRef.current);
  }, [keystroke]);

  // Idle life: blinks and glances, paused with the ambient gate.
  useEffect(() => {
    const root = rootRef.current;
    if (!root || !lively) return;
    const own = timers.current ?? new Timers();
    const life = new LifeScheduler({
      blink: () => motion.blink(root, own),
      glance: (offset) => motion.glance(root, offset),
    });
    const sync = () => {
      const snapshot = ambient?.snapshot();
      if (!snapshot || snapshot.running) life.start();
      else life.stop();
    };
    sync();
    const unsubscribe = ambient?.subscribe(sync);
    return () => { unsubscribe?.(); life.stop(); };
  }, [lively, ambient]);

  // Decorative CSS loops pause while the presence is scrolled out of view.
  useEffect(() => {
    const root = rootRef.current;
    return root ? observeOffscreen(root) : undefined;
  }, []);

  // Pointer: the eyes follow, the body tilts; close by, Pegoles becomes aware.
  useEffect(() => {
    const root = rootRef.current;
    if (!interactive || reducedMotion || !root) return;
    let frame = 0;
    let at: { x: number; y: number } | null = null;
    let aware = false;
    const apply = () => {
      frame = 0;
      let pose: PointerPose | null = null;
      let near = false;
      if (at) {
        const rect = root.getBoundingClientRect();
        const dx = at.x - (rect.left + rect.width / 2);
        const dy = at.y - (rect.top + rect.height / 2);
        pose = pointerPose(dx, dy);
        const distance = Math.hypot(dx, dy);
        near = aware ? distance < AWARE_OUT_PX : distance < AWARE_IN_PX;
      }
      motion.pointer(root, pose);
      if (near !== aware) {
        aware = near;
        root.toggleAttribute("data-aware", aware);
      }
    };
    const move = (event: PointerEvent) => {
      at = { x: event.clientX, y: event.clientY };
      if (!frame) frame = window.requestAnimationFrame(apply);
    };
    const leave = () => {
      at = null;
      if (!frame) frame = window.requestAnimationFrame(apply);
    };
    window.addEventListener("pointermove", move, { passive: true });
    document.documentElement.addEventListener("pointerleave", leave);
    return () => {
      window.removeEventListener("pointermove", move);
      document.documentElement.removeEventListener("pointerleave", leave);
      if (frame) window.cancelAnimationFrame(frame);
      motion.pointer(root, null);
      root.removeAttribute("data-aware");
    };
  }, [interactive, reducedMotion]);

  const press = () => {
    const root = rootRef.current;
    if (root) motion.boop(root, timers.current ?? new Timers(), reducedMotion);
    onPress?.();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (!event.repeat) press();
  };

  const loopClass = GESTURE_MODES.has(mode) ? "" : WORK_MODES.has(mode) ? "pg-work-anim" : "pg-ambient";
  const style = {
    ...poseVariables(targets),
    "--presence-size": `${size}px`,
    "--p-look-x": look?.x ?? 0,
    "--p-look-y": look?.y ?? 0,
    "--p-ring-mask": MARK_RING_MASK,
  } as CSSProperties;
  const a11y = pressable
    ? { role: "button", tabIndex: 0, "aria-label": label ?? "Pegoles", onClick: press, onKeyDown }
    : decorative
      ? { "aria-hidden": true as const }
      : { role: "img", "aria-label": label ?? PRESENCE_LABEL[mode] };

  return <div ref={rootRef} className={className ? `presence ${className}` : "presence"} data-mode={mode} data-small={size < 28 || undefined}
    data-pressable={pressable || undefined} style={style} {...a11y}>
    <PresenceArt size={size} pulse={ripple} field={showField} loopClass={loopClass} />
  </div>;
}

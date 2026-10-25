/**
 * Imperative reactions of the presence: custom properties written on the
 * presence root and short Web Animations. No React state, so a blink or
 * a keystroke never re-renders anything. Every call is safe where the Web
 * Animations API is missing (tests): the pose still changes, only the
 * one-shot flourish is skipped.
 */
import type { Offset, Timers } from "./life";
import type { PresenceMode } from "./modes";

/** Where the person's pointer is, as an eye offset (fraction of the mark) and a tilt (degrees). */
export interface PointerPose {
  readonly eyeX: number;
  readonly eyeY: number;
  readonly pitch: number;
  readonly yaw: number;
}

const OUT = "cubic-bezier(0.23, 1, 0.32, 1)";
const MOVE = "cubic-bezier(0.65, 0, 0.35, 1)";
const LIGHT = "cubic-bezier(0.37, 0, 0.63, 1)";

function animate(element: Element | null, keyframes: Keyframe[], options: KeyframeAnimationOptions): void {
  if (element && typeof (element as HTMLElement).animate === "function") (element as HTMLElement).animate(keyframes, options);
}

function set(root: HTMLElement, name: string, value: string | null): void {
  if (value === null) root.style.removeProperty(name);
  else root.style.setProperty(name, value);
}

export function blink(root: HTMLElement, timers: Timers): void {
  set(root, "--p-lid-dur", "70ms");
  set(root, "--p-blink", "0.92");
  timers.after(85, () => {
    set(root, "--p-lid-dur", "130ms");
    set(root, "--p-blink", null);
    timers.after(150, () => set(root, "--p-lid-dur", null));
  });
}

export function glance(root: HTMLElement, offset: Offset | null): void {
  set(root, "--p-glance-x", offset ? offset.x.toFixed(4) : null);
  set(root, "--p-glance-y", offset ? offset.y.toFixed(4) : null);
}

export function pointer(root: HTMLElement, pose: PointerPose | null): void {
  set(root, "--p-point-x", pose ? pose.eyeX.toFixed(4) : null);
  set(root, "--p-point-y", pose ? pose.eyeY.toFixed(4) : null);
  set(root, "--p-tilt-x", pose ? `${(-pose.pitch).toFixed(2)}deg` : null);
  set(root, "--p-tilt-y", pose ? `${pose.yaw.toFixed(2)}deg` : null);
}

export function ripple(root: HTMLElement, strength: number): void {
  animate(root.querySelector(".presence__shimmer"), [
    { opacity: 0.7 * strength, transform: "scale(1.01)" },
    { opacity: 0, transform: `scale(${1 + 0.4 * strength})` },
  ], { duration: 420 + 340 * strength, easing: OUT });
}

/** A keystroke: eyes widen for a moment, a lean toward `look`, a light shimmer. */
export function nudge(root: HTMLElement, look: Offset | null, timers: Timers, reducedMotion: boolean): void {
  set(root, "--p-kick", "1");
  if (!reducedMotion) {
    set(root, "--p-lean-x", String(look?.x ?? 0));
    set(root, "--p-lean-y", String(look?.y ?? 1));
  }
  timers.after(120, () => {
    set(root, "--p-kick", null);
    set(root, "--p-lean-x", null);
    set(root, "--p-lean-y", null);
  });
  ripple(root, 0.35);
}

/** The press: squash and stretch, a happy squint, one ripple. About 450 ms. */
export function boop(root: HTMLElement, timers: Timers, reducedMotion: boolean): void {
  if (reducedMotion) {
    animate(root.querySelector(".presence__flash"), [{ opacity: 0 }, { opacity: 0.9, offset: 0.25 }, { opacity: 0 }], { duration: 360, easing: LIGHT });
    return;
  }
  animate(root.querySelector(".presence__react"), [
    { transform: "scale(1, 1)", easing: "cubic-bezier(0.3, 0, 0.2, 1)" },
    { transform: "scale(1.05, 0.92)", offset: 0.2, easing: OUT },
    { transform: "scale(1, 1)" },
  ], { duration: 460 });
  set(root, "--p-lid-dur", "110ms");
  set(root, "--p-squint", "0.6");
  timers.after(170, () => {
    set(root, "--p-lid-dur", "260ms");
    set(root, "--p-squint", null);
    timers.after(280, () => set(root, "--p-lid-dur", null));
  });
  ripple(root, 0.8);
}

/** One-shot gestures that answer a change of mode (never the mode it mounted in). */
export function gesture(root: HTMLElement, mode: PresenceMode, reducedMotion: boolean): void {
  const react = root.querySelector(".presence__react");
  if (mode === "done") {
    animate(root.querySelector(".presence__bloom"), [{ opacity: 0 }, { opacity: 0.85, offset: 0.3 }, { opacity: 0 }], { duration: 320, easing: LIGHT });
    if (!reducedMotion) animate(react, [{ transform: "scale(1)" }, { transform: "scale(1.028)", offset: 0.38 }, { transform: "scale(1)" }], { duration: 1100, easing: LIGHT });
    return;
  }
  if (reducedMotion) return;
  if (mode === "acknowledging") {
    animate(react, [
      { transform: "scale(1) translateY(0)" },
      { transform: "scale(1.035) translateY(1.2%)", offset: 0.32 },
      { transform: "scale(1) translateY(0)" },
    ], { duration: 720, easing: OUT });
    ripple(root, 0.9);
  } else if (mode === "error") {
    animate(react, [
      { transform: "translateX(0)" },
      { transform: "translateX(-1.4%)", offset: 0.3 },
      { transform: "translateX(0.6%)", offset: 0.62 },
      { transform: "translateX(0)" },
    ], { duration: 460, easing: MOVE });
  }
}

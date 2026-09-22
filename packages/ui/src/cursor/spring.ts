/**
 * Damped spring integrator (pure), parametrized like Apple's springs:
 * `response` (s) = period of the undamped oscillation, `dampingRatio` =
 * 1 critically damped (no overshoot), < 1 bouncy, > 1 sluggish.
 *
 * Uses the exact closed-form solution, so it is frame-rate independent
 * and unconditionally stable: one 100 ms step lands exactly where six
 * 16.7 ms steps do. X and Y are integrated as independent axes.
 */
import { spring as springTokens } from "../tokens/motion.js";

export interface SpringConfig {
  /** Seconds. Lower = snappier. */
  readonly response: number;
  /** 1 = critically damped. */
  readonly dampingRatio: number;
}

export interface SpringAxis {
  readonly position: number;
  readonly velocity: number;
}

/** Agent cursor travel: the shared `pointer` motion token (arrives decisively, no wobble). */
export const AGENT_CURSOR_SPRING: SpringConfig = {
  response: springTokens.pointer.visualDuration,
  dampingRatio: 1 - springTokens.pointer.bounce,
};

/** Within this distance (px) and speed (px/s) a spring counts as settled. */
export const SETTLE_DISTANCE = 0.05;
export const SETTLE_SPEED = 2;

export function stepSpring(axis: SpringAxis, target: number, config: SpringConfig, dt: number): SpringAxis {
  if (!(dt > 0)) return axis;
  const omega = (2 * Math.PI) / Math.max(config.response, 1e-3);
  const zeta = Math.max(config.dampingRatio, 0);
  const x0 = axis.position - target;
  const v0 = axis.velocity;
  let x: number;
  let v: number;
  if (Math.abs(zeta - 1) < 1e-6) {
    const e = Math.exp(-omega * dt);
    const b = v0 + omega * x0;
    x = (x0 + b * dt) * e;
    v = (v0 - omega * b * dt) * e;
  } else if (zeta < 1) {
    const wd = omega * Math.sqrt(1 - zeta * zeta);
    const e = Math.exp(-zeta * omega * dt);
    const c = Math.cos(wd * dt);
    const s = Math.sin(wd * dt);
    const b = (v0 + zeta * omega * x0) / wd;
    x = e * (x0 * c + b * s);
    v = e * (-zeta * omega * (x0 * c + b * s) + wd * (b * c - x0 * s));
  } else {
    const r = omega * Math.sqrt(zeta * zeta - 1);
    const r1 = -zeta * omega + r;
    const r2 = -zeta * omega - r;
    const c2 = (v0 - r1 * x0) / (r2 - r1);
    const c1 = x0 - c2;
    const e1 = Math.exp(r1 * dt);
    const e2 = Math.exp(r2 * dt);
    x = c1 * e1 + c2 * e2;
    v = c1 * r1 * e1 + c2 * r2 * e2;
  }
  return { position: target + x, velocity: v };
}

export function isSettled(axis: SpringAxis, target: number): boolean {
  return Math.abs(axis.position - target) < SETTLE_DISTANCE && Math.abs(axis.velocity) < SETTLE_SPEED;
}

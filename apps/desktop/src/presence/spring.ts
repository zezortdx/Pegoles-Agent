/**
 * Critically damped spring (exact closed form): frame-rate independent and
 * unconditionally stable, so one 100 ms step lands where six 16.7 ms steps
 * do. No overshoot, ever — Pegoles never bounces.
 */
export interface Spring {
  position: number;
  velocity: number;
}

export function stepCritical(spring: Spring, target: number, response: number, dt: number): void {
  if (!(dt > 0)) return;
  const omega = (2 * Math.PI) / Math.max(response, 1e-3);
  const x0 = spring.position - target;
  const v0 = spring.velocity;
  const e = Math.exp(-omega * dt);
  const b = v0 + omega * x0;
  spring.position = target + (x0 + b * dt) * e;
  spring.velocity = (v0 - omega * b * dt) * e;
}

/** Settled within `epsilon` of the target with negligible speed. */
export function isSettled(spring: Spring, target: number, epsilon = 1e-3): boolean {
  return Math.abs(spring.position - target) < epsilon && Math.abs(spring.velocity) < epsilon * 4;
}

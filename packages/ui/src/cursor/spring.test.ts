import { describe, expect, it } from "vitest";
import { AGENT_CURSOR_SPRING, isSettled, stepSpring, type SpringAxis, type SpringConfig } from "./spring.js";

function simulate(config: SpringConfig, from: SpringAxis, target: number, dt: number, seconds: number): SpringAxis[] {
  const out: SpringAxis[] = [];
  let axis = from;
  for (let t = 0; t < seconds - 1e-9; t += dt) {
    axis = stepSpring(axis, target, config, dt);
    out.push(axis);
  }
  return out;
}

describe("stepSpring", () => {
  const critical: SpringConfig = { response: 0.35, dampingRatio: 1 };

  it("converges to the target (critically damped)", () => {
    const path = simulate(critical, { position: 0, velocity: 0 }, 800, 1 / 60, 1.5);
    const last = path[path.length - 1] as SpringAxis;
    expect(Math.abs(last.position - 800)).toBeLessThan(0.01);
    expect(isSettled(last, 800)).toBe(true);
  });

  it("never overshoots when critically damped from rest", () => {
    const path = simulate(critical, { position: 0, velocity: 0 }, 500, 1 / 120, 2);
    let prev = 0;
    for (const p of path) {
      expect(p.position).toBeLessThanOrEqual(500 + 1e-9);
      expect(p.position).toBeGreaterThanOrEqual(prev - 1e-9);
      prev = p.position;
    }
  });

  it("is frame-rate independent (exact closed form)", () => {
    const at60 = simulate(critical, { position: 0, velocity: 300 }, 400, 1 / 60, 0.5);
    const at120 = simulate(critical, { position: 0, velocity: 300 }, 400, 1 / 120, 0.5);
    const a = at60[at60.length - 1] as SpringAxis;
    const b = at120[at120.length - 1] as SpringAxis;
    expect(a.position).toBeCloseTo(b.position, 6);
    expect(a.velocity).toBeCloseTo(b.velocity, 5);
  });

  it("stays stable for a huge time step (tab came back)", () => {
    const axis = stepSpring({ position: 0, velocity: 5000 }, 100, critical, 5);
    expect(Number.isFinite(axis.position)).toBe(true);
    expect(Math.abs(axis.position - 100)).toBeLessThan(1e-6);
  });

  it("underdamped and overdamped branches also converge", () => {
    for (const dampingRatio of [0.6, 0.96, 1.4]) {
      const path = simulate({ response: 0.3, dampingRatio }, { position: 0, velocity: 0 }, 200, 1 / 60, 3);
      expect(Math.abs((path[path.length - 1] as SpringAxis).position - 200)).toBeLessThan(0.05);
    }
  });

  it("the agent cursor spring (motion token) arrives within ~0.6 s without visible overshoot", () => {
    const path = simulate(AGENT_CURSOR_SPRING, { position: 0, velocity: 0 }, 1000, 1 / 60, 0.6);
    const peak = Math.max(...path.map((p) => p.position));
    expect(peak - 1000).toBeLessThan(0.5);
    expect(Math.abs((path[path.length - 1] as SpringAxis).position - 1000)).toBeLessThan(2);
  });

  it("ignores non-positive dt", () => {
    const axis = { position: 3, velocity: 4 };
    expect(stepSpring(axis, 10, critical, 0)).toBe(axis);
    expect(stepSpring(axis, 10, critical, -1)).toBe(axis);
  });
});

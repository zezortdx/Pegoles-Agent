import { describe, expect, it } from "vitest";
import { AvatarController, engineStateForMode } from "./controller";
import type { PresenceMode } from "./modes";

const cases: [PresenceMode, string][] = [
  ["offline", "idle"], ["idle", "idle"], ["attentive", "wide"],
  ["acknowledging", "idle"], ["thinking", "thinking"], ["planning", "thinking"],
  ["working", "orbit"], ["using-computer", "orbit"], ["waiting", "notify"],
  ["needs-user", "notify"], ["blocked", "exclaim"], ["done", "idle"], ["error", "exclaim"],
];

describe("Core presence mode to avatar engine", () => {
  it.each(cases)("maps %s to %s", (mode, expected) => {
    expect(engineStateForMode(mode, 152)).toBe(expected);
    expect(new AvatarController(mode, 152).engineState).toBe(expected);
  });

  it.each([16, 20, 40, 52, 55])("uses thinking for active work at %i px", (size) => {
    for (const mode of ["working", "using-computer"] as const) {
      const avatar = new AvatarController(mode, size);
      expect(engineStateForMode(mode, size)).toBe("thinking");
      expect(avatar.staticFrame().arcs).toHaveLength(0);
      for (let i = 0; i < 100; i++) {
        expect(avatar.sample(0.064).arcs).toHaveLength(0);
        expect(avatar.engineState).toBe("thinking");
      }
    }
  });

  it("keeps the orbit at the 56 px boundary", () => {
    expect(engineStateForMode("using-computer", 56)).toBe("orbit");
    expect(new AvatarController("using-computer", 56).staticFrame().arcs.length).toBeGreaterThan(0);
  });

  it("keeps transition frames finite when clearing the gaze", () => {
    const avatar = new AvatarController("idle", 152);
    avatar.setLook({ x: 0.5, y: -0.5 });
    avatar.sample(0.1);
    for (const mode of ["attentive", "using-computer", "needs-user", "idle"] as const) {
      avatar.setMode(mode, 152);
      expect(JSON.stringify(avatar.sample(0))).not.toMatch(/NaN|Infinity/);
    }
    avatar.react("acknowledge");
    avatar.settle();
    expect(avatar.engineState).toBe("idle");
    expect(JSON.stringify(avatar.sample(0))).not.toMatch(/NaN|Infinity/);
  });

  it("cycles real work without changing the app mode", () => {
    const avatar = new AvatarController("working", 152);
    avatar.sample(3.5);
    expect(avatar.engineState).toBe("idle");
    avatar.sample(1);
    expect(avatar.engineState).toBe("orbit");
  });

  it("cancels one-shot reactions when Core changes mode", () => {
    const avatar = new AvatarController("idle", 152);
    avatar.react("acknowledge");
    expect(avatar.engineState).toBe("wink");
    avatar.setMode("thinking", 152);
    expect(avatar.engineState).toBe("thinking");
    avatar.sample(3);
    expect(avatar.engineState).toBe("thinking");
  });

  it.each(["offline", "waiting", "needs-user", "blocked", "error"] as const)("drops active work immediately for %s", (mode) => {
    const avatar = new AvatarController("using-computer", 152);
    avatar.sample(1.2);
    avatar.setMode(mode, 152);
    expect(avatar.sample(0).arcs).toHaveLength(0);
    expect(avatar.engineState).toBe(engineStateForMode(mode, 152));
    avatar.react("acknowledge");
    expect(avatar.engineState).toBe(engineStateForMode(mode, 152));
  });

  it("drops the orbit immediately when resized below 56 px", () => {
    const avatar = new AvatarController("working", 152);
    avatar.sample(1.2);
    avatar.setMode("working", 55);
    expect(avatar.engineState).toBe("thinking");
    expect(avatar.sample(0).arcs).toHaveLength(0);
  });

  it("plays a reaction once and returns to the current mode", () => {
    const avatar = new AvatarController("idle", 152);
    avatar.react("acknowledge");
    avatar.sample(1);
    expect(avatar.engineState).toBe("wink");
    avatar.sample(0.7);
    expect(avatar.engineState).toBe("idle");
    avatar.sample(2);
    expect(avatar.engineState).toBe("idle");
  });

  it("samples a deterministic static pose of the current mode", () => {
    const avatar = new AvatarController("working", 152);
    const first = avatar.staticFrame();
    avatar.sample(0.8);
    avatar.react("acknowledge");
    expect(avatar.staticFrame()).toEqual(first);
    avatar.setMode("needs-user", 152);
    expect(avatar.staticFrame().notification).toMatchObject({ r: 15 });
    expect(avatar.staticFrame().arcs).toHaveLength(0);
  });
});

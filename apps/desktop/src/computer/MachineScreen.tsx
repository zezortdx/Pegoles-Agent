import type { CSSProperties } from "react";
import type { ComputerModel, ComputerPhase } from "../state/computerModel";

/**
 * How lit Pegoles Computer's drawn screen is. One vocabulary everywhere the
 * machine appears (sidebar, title bar, task brief, Home, the panel), all
 * derived from the real computer phase: never a timer, never a guess.
 */
export type MachineLight = "offline" | "setup" | "off" | "waking" | "ready" | "agent" | "user" | "paused" | "error";

const LIGHT: Record<ComputerPhase, MachineLight> = {
  unavailable: "offline",
  "needs-setup": "setup",
  off: "off",
  starting: "waking",
  ready: "ready",
  agent: "agent",
  user: "user",
  paused: "paused",
  stopping: "waking",
  error: "error",
};

export function machineLight(model: ComputerModel): MachineLight {
  return LIGHT[model.phase];
}

/** 0..1: follows the real boot stages. */
export function wakeLevel(model: ComputerModel): number {
  if (model.phase === "starting") {
    const done = model.steps.filter((step) => step.state === "done").length;
    return 0.3 + 0.22 * done;
  }
  if (model.phase === "stopping") return 0.2;
  return 0;
}

export interface MachineScreenProps {
  readonly model: ComputerModel;
  /** chip ≈ 22×14 (navigation), card ≈ 64×40 (brief, Home). */
  readonly size: "chip" | "card";
  readonly className?: string;
}

/**
 * The computer, drawn small: a bezel, a glass, and the light it gives off.
 * Dark when off; brightens through real boot stages; blue while Pegoles
 * uses it, amber while you do. Decorative: always next to words that say it.
 */
export function MachineScreen({ model, size, className }: MachineScreenProps) {
  const light = machineLight(model);
  const style = { "--wake": wakeLevel(model) } as CSSProperties;
  return (
    <span className={className ? `machine machine--${size} ${className}` : `machine machine--${size}`} data-light={light} style={style} aria-hidden="true">
      <span className="machine__glass">
        <span className="machine__glow" />
        {light === "waking" && <span className="machine__scan pg-work-anim" />}
        {light === "agent" && <span className="machine__trace pg-work-anim" />}
        {light === "paused" && <span className="machine__pause" />}
      </span>
    </span>
  );
}

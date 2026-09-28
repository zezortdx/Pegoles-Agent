import type { CSSProperties } from "react";
import { MARK_RING_MASK } from "@pegoles/ui";
import { GESTURE_MODES, PRESENCE_LABEL, WORK_MODES, type PresenceMode } from "./modes";
import { PresenceArt } from "./PresenceArt";
import { poseVariables, presenceTargets } from "./targets";
import "./presence.css";

export interface PresenceMarkProps {
  readonly mode: PresenceMode;
  /** CSS px (16–36: sidebar brand, hand-off chips, the Thought Ribbon). */
  readonly size: number;
  /** aria-hidden by default: marks sit next to text that already says it. */
  readonly decorative?: boolean;
  readonly label?: string;
  readonly className?: string;
}

/**
 * The same mark as PegolesPresence, for 16–36 px spots: no idle-life
 * timers, no pointer listeners, no thought field.
 * Its only continuous motion is the mode's CSS loop, gated like every other
 * presence loop (pg-work-anim while working, pg-ambient otherwise).
 */
export function PresenceMark({ mode, size, decorative = true, label, className }: PresenceMarkProps) {
  const targets = presenceTargets(mode);
  const loopClass = GESTURE_MODES.has(mode) ? "" : WORK_MODES.has(mode) ? "pg-work-anim" : "pg-ambient";
  const style = {
    ...poseVariables(targets),
    "--presence-size": `${size}px`,
    "--p-ring-mask": MARK_RING_MASK,
  } as CSSProperties;
  const a11y = decorative ? { "aria-hidden": true as const } : { role: "img", "aria-label": label ?? PRESENCE_LABEL[mode] };
  return <div className={className ? `presence presence--mark ${className}` : "presence presence--mark"} data-mode={mode}
    data-small={size < 28 || undefined} style={style} {...a11y}>
    <PresenceArt size={size} pulse={0} field={false} loopClass={loopClass} />
  </div>;
}

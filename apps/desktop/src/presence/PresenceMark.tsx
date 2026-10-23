import type { CSSProperties } from "react";
import { PRESENCE_LABEL, type PresenceMode } from "./modes";
import { PresenceAvatar } from "./PresenceAvatar";
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

/** Compact avatar; the controller chooses a readable pose below 56 px. */
export function PresenceMark({ mode, size, decorative = true, label, className }: PresenceMarkProps) {
  const a11y = decorative ? { "aria-hidden": true as const } : { role: "img", "aria-label": label ?? PRESENCE_LABEL[mode] };
  return <div className={className ? `presence presence--mark ${className}` : "presence presence--mark"}
    data-mode={mode} data-small={size < 28 || undefined} style={{ "--presence-size": `${size}px` } as CSSProperties} {...a11y}>
    <PresenceAvatar mode={mode} size={size} field={false} decorative />
  </div>;
}

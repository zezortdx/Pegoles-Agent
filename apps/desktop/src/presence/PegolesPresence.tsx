import { useState, type CSSProperties, type KeyboardEvent } from "react";
import { PRESENCE_LABEL } from "./modes";
import type { PresenceMode } from "./modes";
import { PresenceAvatar } from "./PresenceAvatar";
import "./presence.css";

export { pointerPose } from "./PresenceAvatar";

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
  /** Hero only: the eyes follow the pointer. */
  interactive?: boolean;
  /** Hero only: a real button; pressing it gets a one-shot acknowledgement. */
  pressable?: boolean;
  /** Called after a press, in addition to the acknowledgement. */
  onPress?: () => void;
  /** Allow thought contours beyond the mark. Default: size ≥ 40. */
  field?: boolean;
  /** aria-hidden when true; otherwise role="img" with a state label. */
  decorative?: boolean;
  /** Overrides the default sentence for the mode. */
  label?: string;
  className?: string;
}

/** Existing shell API; state derivation stays in the Core visual adapter. */
export function PegolesPresence({ mode, size, look = null, pulse = 0, nudge = 0, interactive = false,
  pressable = false, onPress, field, decorative = false, label, className }: PegolesPresenceProps) {
  const [presses, setPresses] = useState(0);
  const press = () => {
    setPresses((count) => count + 1);
    onPress?.();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (!event.repeat) press();
  };
  const a11y = pressable
    ? { role: "button", tabIndex: 0, "aria-label": label ?? PRESENCE_LABEL[mode], onClick: press, onKeyDown }
    : decorative
      ? { "aria-hidden": true as const }
      : { role: "img", "aria-label": label ?? PRESENCE_LABEL[mode] };

  return <div className={className ? `presence ${className}` : "presence"} data-mode={mode}
    data-small={size < 28 || undefined} data-pressable={pressable || undefined}
    style={{ "--presence-size": `${size}px` } as CSSProperties} {...a11y}>
    <PresenceAvatar mode={mode} size={size} look={look} pulse={pulse} nudge={nudge}
      interactive={interactive} reaction={presses} field={field} decorative />
  </div>;
}

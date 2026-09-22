import type { HTMLAttributes } from "react";
import { cx } from "./cx.js";
import "./status.css";

/**
 * Status tones. Each tone has its own SHAPE and its own color, and the
 * indicator always carries a text label — color is never the only cue.
 * Blue appears only for `active` (Pegoles present and working).
 */
export type StatusTone = "neutral" | "active" | "success" | "warning" | "danger" | "waiting" | "paused" | "user";

export const STATUS_TONES: readonly StatusTone[] = [
  "neutral",
  "active",
  "success",
  "warning",
  "danger",
  "waiting",
  "paused",
  "user",
];

/** Shape vocabulary (documented in DESIGN_SYSTEM.md). */
export const STATUS_SHAPES: Readonly<Record<StatusTone, string>> = {
  neutral: "ring",
  active: "octagon",
  success: "disc-check",
  warning: "triangle",
  danger: "diamond",
  waiting: "half-disc",
  paused: "bars",
  user: "ring-dot",
};

function Shape({ tone }: { readonly tone: StatusTone }) {
  switch (tone) {
    case "active":
      return (
        <path
          d="M4.6 1.5h2.8a1 1 0 0 1 .7.3l2.1 2.1a1 1 0 0 1 .3.7v2.8a1 1 0 0 1-.3.7l-2.1 2.1a1 1 0 0 1-.7.3H4.6a1 1 0 0 1-.7-.3L1.8 8.1a1 1 0 0 1-.3-.7V4.6a1 1 0 0 1 .3-.7l2.1-2.1a1 1 0 0 1 .7-.3Z"
          fill="currentColor"
        />
      );
    case "success":
      return (
        <>
          <circle cx="6" cy="6" r="5" fill="currentColor" />
          <path
            d="m3.7 6.1 1.6 1.6 3-3.2"
            fill="none"
            stroke="var(--pg-bg-primary)"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </>
      );
    case "warning":
      return <path d="M6 1.4a.9.9 0 0 1 .78.45l4.3 7.5a.9.9 0 0 1-.78 1.35H1.7a.9.9 0 0 1-.78-1.35l4.3-7.5A.9.9 0 0 1 6 1.4Z" fill="currentColor" />;
    case "danger":
      return <rect x="2.3" y="2.3" width="7.4" height="7.4" rx="1.2" transform="rotate(45 6 6)" fill="currentColor" />;
    case "waiting":
      return (
        <>
          <circle cx="6" cy="6" r="4.4" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <path d="M6 1.6a4.4 4.4 0 0 1 0 8.8Z" fill="currentColor" />
        </>
      );
    case "paused":
      return (
        <>
          <rect x="2.6" y="2" width="2.4" height="8" rx="0.9" fill="currentColor" />
          <rect x="7" y="2" width="2.4" height="8" rx="0.9" fill="currentColor" />
        </>
      );
    case "user":
      return (
        <>
          <circle cx="6" cy="6" r="4.4" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <circle cx="6" cy="6" r="2" fill="currentColor" />
        </>
      );
    case "neutral":
      return <circle cx="6" cy="6" r="4.3" fill="none" stroke="currentColor" strokeWidth="1.4" />;
  }
}

export interface StatusIndicatorProps extends Omit<HTMLAttributes<HTMLSpanElement>, "children"> {
  readonly tone: StatusTone;
  /** Always required: the words that carry the state. */
  readonly label: string;
  /** Optional secondary detail, e.g. "4.2 s" or "since 14:02". */
  readonly detail?: string;
  readonly size?: "sm" | "md";
  /** Visually hide the label (it stays in the accessibility tree). */
  readonly hideLabel?: boolean;
  /** Announce changes politely (role="status"). */
  readonly live?: boolean;
  /** Presence halo on `active` (ambient: sleeps with the idle gate). */
  readonly pulse?: boolean;
  /** Pill background (for headers and chips). */
  readonly pill?: boolean;
}

export function StatusIndicator({
  tone,
  label,
  detail,
  size = "md",
  hideLabel = false,
  live = false,
  pulse = false,
  pill = false,
  className,
  ...rest
}: StatusIndicatorProps) {
  return (
    <span
      className={cx("pg-status", className)}
      data-tone={tone}
      data-shape={STATUS_SHAPES[tone]}
      data-size={size}
      data-pill={pill ? "true" : undefined}
      role={live ? "status" : undefined}
      aria-live={live ? "polite" : undefined}
      {...rest}
    >
      <span className="pg-status__glyph" aria-hidden="true">
        {pulse && tone === "active" && <span className="pg-status__halo pg-ambient" />}
        <svg viewBox="0 0 12 12" width="12" height="12" focusable="false">
          <Shape tone={tone} />
        </svg>
      </span>
      <span className={hideLabel ? "pg-visually-hidden" : "pg-status__label"}>{label}</span>
      {detail && !hideLabel && <span className="pg-status__detail">{detail}</span>}
    </span>
  );
}

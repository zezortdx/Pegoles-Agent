import type { HTMLAttributes, ReactNode } from "react";
import {
  AlertIcon,
  InfoIcon,
  MonitorIcon,
  PackageIcon,
  PointerIcon,
  PresenceIcon,
  TaskIcon,
  TerminalIcon,
  WindowIcon,
  type IconProps,
} from "./icons.js";
import type { StatusTone } from "./StatusIndicator.js";
import { cx } from "./cx.js";
import "./activity.css";

/** What an activity row is about. Each kind has its own glyph. */
export type ActivityKind =
  | "task"
  | "agent"
  | "computer"
  | "guest"
  | "display"
  | "control"
  | "image"
  | "error"
  | "info";

const KIND_ICON: Readonly<Record<ActivityKind, (props: IconProps) => ReactNode>> = {
  task: TaskIcon,
  agent: PresenceIcon,
  computer: MonitorIcon,
  guest: TerminalIcon,
  display: WindowIcon,
  control: PointerIcon,
  image: PackageIcon,
  error: AlertIcon,
  info: InfoIcon,
};

const KIND_TONE: Readonly<Record<ActivityKind, StatusTone>> = {
  task: "neutral",
  agent: "active",
  computer: "neutral",
  guest: "neutral",
  display: "neutral",
  control: "user",
  image: "neutral",
  error: "danger",
  info: "neutral",
};

export interface ActivityItemProps extends Omit<HTMLAttributes<HTMLLIElement>, "title"> {
  readonly kind: ActivityKind;
  /** One line, past tense or present state: "Debian ready". */
  readonly title: string;
  /** Optional supporting line: reason, duration, identifier. */
  readonly detail?: string;
  /** Machine time for <time dateTime>. */
  readonly at: Date | string;
  /** Human time label; defaults to HH:MM in the user's locale. */
  readonly timeLabel?: string;
  /** Overrides the kind's default tone (e.g. success on "ready"). */
  readonly tone?: StatusTone;
  /**
   * Identical consecutive events coalesced into this row (×N). Coalesce
   * repeats instead of rendering heartbeat spam.
   */
  readonly repeatCount?: number;
  /** Quiet rows recede (routine lifecycle); default rows read normally. */
  readonly emphasis?: "normal" | "quiet";
  /** Newest live row: subtle Pegoles-blue energy, settles when superseded. */
  readonly current?: boolean;
}

function toDate(at: Date | string): Date | null {
  const d = at instanceof Date ? at : new Date(at);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** One row of the activity timeline. Render inside <ActivityList>. */
export function ActivityItem({
  kind,
  title,
  detail,
  at,
  timeLabel,
  tone,
  repeatCount,
  emphasis = "normal",
  current = false,
  className,
  ...rest
}: ActivityItemProps) {
  const Icon = KIND_ICON[kind];
  const date = toDate(at);
  const label =
    timeLabel ?? (date ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "");
  const repeats = repeatCount !== undefined && repeatCount > 1 ? repeatCount : null;
  return (
    <li
      className={cx("pg-activity", className)}
      data-kind={kind}
      data-tone={tone ?? KIND_TONE[kind]}
      data-emphasis={emphasis}
      data-current={current ? "true" : undefined}
      {...rest}
    >
      <span className="pg-activity__glyph" aria-hidden="true">
        <Icon size={14} />
      </span>
      <div className="pg-activity__text">
        <p className="pg-activity__title">
          {title}
          {repeats !== null && (
            <span className="pg-activity__repeat">
              <span aria-hidden="true">×{repeats}</span>
              <span className="pg-visually-hidden">, {repeats} times</span>
            </span>
          )}
        </p>
        {detail && <p className="pg-activity__detail">{detail}</p>}
      </div>
      {date && (
        <time className="pg-activity__time" dateTime={date.toISOString()}>
          {label}
        </time>
      )}
    </li>
  );
}

export interface ActivityListProps extends HTMLAttributes<HTMLOListElement> {
  /** Accessible name, e.g. "Computer activity". */
  readonly label: string;
}

/** Ordered timeline container (newest first is the app's choice). */
export function ActivityList({ label, className, children, ...rest }: ActivityListProps) {
  return (
    <ol className={cx("pg-activity-list", className)} aria-label={label} {...rest}>
      {children}
    </ol>
  );
}

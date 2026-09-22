import { cx } from "./cx.js";
import "./progress.css";

export interface ProgressLineProps {
  /**
   * Real completion fraction 0..1, or null when no real measure exists —
   * then the line is indeterminate. Never pass a guessed number.
   */
  readonly value: number | null;
  /** Accessible name, e.g. "Downloading Pegoles Base Image". */
  readonly label: string;
  /** Render the percentage as text (only meaningful with a real value). */
  readonly showValue?: boolean;
  readonly className?: string;
}

export function clampFraction(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

/**
 * Hairline progress. Indeterminate = a light sweeping along the line
 * (work animation: runs while visible; opacity pulse under reduced
 * motion; static dashed line on Minimal). Determinate = transform
 * scaleX, with an optional percentage label.
 */
export function ProgressLine({ value, label, showValue = false, className }: ProgressLineProps) {
  if (value === null) {
    return (
      <span className={cx("pg-progress", className)} data-mode="indeterminate" role="progressbar" aria-label={label}>
        <span className="pg-progress__sweep pg-work-anim" />
      </span>
    );
  }
  const fraction = clampFraction(value);
  const percent = Math.round(fraction * 100);
  return (
    <span className={cx("pg-progress-row", className)}>
      <span
        className="pg-progress"
        data-mode="determinate"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <span className="pg-progress__fill" style={{ transform: `scaleX(${fraction})` }} />
      </span>
      {showValue && (
        <span className="pg-progress__value" aria-hidden="true">
          {percent}%
        </span>
      )}
    </span>
  );
}

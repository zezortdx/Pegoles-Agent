import type { CSSProperties } from "react";

export interface ProgressMeterProps {
  /** Accessible name ("Setting up Pegoles Local"). */
  readonly label: string;
  /** 0–100 from a real measure; null when there is none (never an invented percentage). */
  readonly percent: number | null;
  /** The measure in words ("940 MB of 2.2 GB"). */
  readonly valueText?: string;
  /** Work goes on past the bar's end (finishing): the fill breathes instead of standing still. */
  readonly working?: boolean;
}

/**
 * A hairline of real progress. The fill scales (compositor only) as Core
 * reports bytes; without a measure a light passes along the track instead.
 */
export function ProgressMeter({ label, percent, valueText, working = false }: ProgressMeterProps) {
  const determinate = percent !== null;
  return (
    <div
      className="meter"
      role="progressbar"
      aria-label={label}
      aria-valuemin={determinate ? 0 : undefined}
      aria-valuemax={determinate ? 100 : undefined}
      aria-valuenow={determinate ? percent : undefined}
      aria-valuetext={valueText}
      data-indeterminate={!determinate || undefined}
      style={determinate ? ({ "--meter": percent / 100 } as CSSProperties) : undefined}
    >
      <span className="meter__fill pg-work-anim" data-working={working || undefined} aria-hidden="true" />
    </div>
  );
}

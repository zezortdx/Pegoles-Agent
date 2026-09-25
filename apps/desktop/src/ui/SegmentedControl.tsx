import { useRef, type KeyboardEvent } from "react";
import { m } from "motion/react";
import { duration, ease } from "../lib/motion";

export interface Segment<T extends string> {
  readonly value: T;
  readonly label: string;
}

interface SegmentedControlProps<T extends string> {
  readonly label: string;
  readonly segments: readonly Segment<T>[];
  readonly value: T;
  readonly onChange: (value: T) => void;
  /** Distinct per instance so two controls never share a sliding thumb. */
  readonly id: string;
  /** "small" for page toolbars; "regular" (default) inside settings rows. */
  readonly size?: "regular" | "small";
}

/**
 * Radio group rendered as a segmented control (WAI-ARIA radio pattern:
 * one tab stop, arrows move and select). The thumb is a shared element
 * that glides to the chosen segment.
 */
export function SegmentedControl<T extends string>({ label, segments, value, onChange, id, size = "regular" }: SegmentedControlProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const index = Math.max(0, segments.findIndex((s) => s.value === value));
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    const edge = event.key === "Home" ? 0 : event.key === "End" ? segments.length - 1 : null;
    if (!step && edge === null) return;
    event.preventDefault();
    const next = edge ?? (index + step + segments.length) % segments.length;
    onChange(segments[next].value);
    refs.current[next]?.focus();
  };
  return (
    <div className="segmented" data-size={size} role="radiogroup" aria-label={label} onKeyDown={move}>
      {segments.map((segment, i) => {
        const checked = i === index;
        return (
          <button
            key={segment.value}
            ref={(el) => { refs.current[i] = el; }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            className="segmented__option"
            onClick={() => onChange(segment.value)}
          >
            {checked && <m.span className="segmented__thumb" layoutId={`segmented-${id}`} transition={{ duration: duration.layout, ease: ease.out }} aria-hidden="true" />}
            <span className="segmented__label">{segment.label}</span>
          </button>
        );
      })}
    </div>
  );
}

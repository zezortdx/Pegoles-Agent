/**
 * Lab-only controls (segmented radio group, switch, section frame).
 * Not part of @pegoles/ui: the design system ships only what the app needs.
 */
import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface SegmentOption<T extends string> {
  readonly value: T;
  readonly label: string;
}

interface SegmentedProps<T extends string> {
  readonly label: string;
  readonly value: T;
  readonly options: readonly SegmentOption<T>[];
  readonly onChange: (value: T) => void;
  readonly size?: "sm" | "md";
}

/** Radio group with roving focus (arrow keys move and select). */
export function Segmented<T extends string>({ label, value, options, onChange, size = "md" }: SegmentedProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const index = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );

  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const delta = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (delta === 0) return;
    event.preventDefault();
    const next = (index + delta + options.length) % options.length;
    const option = options[next];
    if (!option) return;
    onChange(option.value);
    refs.current[next]?.focus();
  };

  return (
    <div className="lab-segmented" role="radiogroup" aria-label={label} data-size={size} onKeyDown={move}>
      {options.map((option, i) => {
        const checked = option.value === value;
        return (
          <button
            key={option.value}
            ref={(node) => {
              refs.current[i] = node;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            className="lab-segmented__option"
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

interface SwitchProps {
  readonly label: string;
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
  readonly hint?: string;
}

export function Switch({ label, checked, onChange, hint }: SwitchProps) {
  return (
    <button type="button" role="switch" aria-checked={checked} className="lab-switch" onClick={() => onChange(!checked)}>
      <span className="lab-switch__text">
        <span className="lab-switch__label">{label}</span>
        {hint && <span className="lab-switch__hint">{hint}</span>}
      </span>
      <span className="lab-switch__track" aria-hidden="true">
        <span className="lab-switch__thumb" />
      </span>
    </button>
  );
}

interface LabSectionProps {
  readonly id: string;
  readonly index: string;
  readonly title: string;
  readonly lead?: ReactNode;
  readonly children: ReactNode;
  readonly owner?: string;
}

export function LabSection({ id, index, title, lead, children, owner }: LabSectionProps) {
  const headingId = `${id}-title`;
  return (
    <section id={id} className="lab-section" aria-labelledby={headingId}>
      <header className="lab-section__header">
        <p className="lab-section__index">
          {index}
          {owner && <span className="lab-section__owner">{owner}</span>}
        </p>
        <h2 id={headingId} className="lab-section__title">
          {title}
        </h2>
        {lead && <p className="lab-section__lead">{lead}</p>}
      </header>
      {children}
    </section>
  );
}

/** Small inline caption that marks simulated content honestly. */
export function SimulatedTag({ children = "Simulated fixture" }: { readonly children?: ReactNode }) {
  return <span className="lab-simulated">{children}</span>;
}

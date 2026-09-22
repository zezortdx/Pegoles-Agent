import {
  GlassButton,
  PauseIcon,
  PlayIcon,
  PointerIcon,
  ReturnIcon,
  StatusIndicator,
  StopIcon,
  STATUS_SHAPES,
  STATUS_TONES,
  type StatusTone,
} from "@pegoles/ui";
import { LabSection } from "../lab/controls";

const TONE_MEANING: Readonly<Record<StatusTone, { label: string; meaning: string }>> = {
  neutral: { label: "Idle", meaning: "Nothing is happening" },
  active: { label: "Working", meaning: "Pegoles is present and working (the only blue status)" },
  success: { label: "Ready", meaning: "Done or ready" },
  warning: { label: "Low disk", meaning: "Needs attention soon" },
  danger: { label: "Failed", meaning: "Something broke" },
  waiting: { label: "Needs you", meaning: "Waiting on your approval or input" },
  paused: { label: "Paused", meaning: "Held on purpose" },
  user: { label: "You're in control", meaning: "A human is driving" },
};

export function ButtonsSection() {
  return (
    <LabSection
      id="lab-buttons"
      index="04"
      title="Buttons"
      lead="Capsules that answer on press — a 3% squeeze in 100 ms, or a brightness change under reduced motion. Tab through: the focus ring is the mark's cyan eye-light."
    >
      <div className="lab-specimen" role="table" aria-label="Button variants and states">
        <div className="lab-specimen__row lab-specimen__row--head" role="row">
          <span role="columnheader">Variant</span>
          <span role="columnheader">Default</span>
          <span role="columnheader">With icon</span>
          <span role="columnheader">Small · icon only</span>
          <span role="columnheader">Disabled</span>
          <span role="columnheader">Working</span>
        </div>
        <div className="lab-specimen__row" role="row">
          <span role="rowheader" className="lab-mono">primary</span>
          <span role="cell"><GlassButton variant="primary">Start computer</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" icon={<PlayIcon size={14} />}>Resume</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" size="sm" iconOnly icon={<PlayIcon size={12} />}>Resume</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" disabled>Start computer</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" isLoading>Starting</GlassButton></span>
        </div>
        <div className="lab-specimen__row" role="row">
          <span role="rowheader" className="lab-mono">secondary</span>
          <span role="cell"><GlassButton>Take control</GlassButton></span>
          <span role="cell"><GlassButton icon={<PointerIcon size={14} />}>Take control</GlassButton></span>
          <span role="cell"><GlassButton size="sm" iconOnly icon={<PauseIcon size={12} />}>Pause</GlassButton></span>
          <span role="cell"><GlassButton disabled>Take control</GlassButton></span>
          <span role="cell"><GlassButton isLoading>Pausing</GlassButton></span>
        </div>
        <div className="lab-specimen__row" role="row">
          <span role="rowheader" className="lab-mono">quiet</span>
          <span role="cell"><GlassButton variant="quiet">Details</GlassButton></span>
          <span role="cell"><GlassButton variant="quiet" icon={<StopIcon size={14} />}>Stop</GlassButton></span>
          <span role="cell"><GlassButton variant="quiet" size="sm" iconOnly icon={<StopIcon size={12} />}>Stop</GlassButton></span>
          <span role="cell"><GlassButton variant="quiet" disabled>Details</GlassButton></span>
          <span role="cell"><GlassButton variant="quiet" isLoading>Stopping</GlassButton></span>
        </div>
        <div className="lab-specimen__row" role="row">
          <span role="rowheader" className="lab-mono">neutral</span>
          <span role="cell"><GlassButton variant="primary" tone="neutral">Return to Pegoles</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" tone="neutral" icon={<ReturnIcon size={14} />}>Return to Pegoles</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" tone="neutral" size="sm" iconOnly icon={<ReturnIcon size={12} />}>Return to Pegoles</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" tone="neutral" disabled>Return to Pegoles</GlassButton></span>
          <span role="cell"><GlassButton variant="primary" tone="neutral" isLoading>Returning</GlassButton></span>
        </div>
      </div>
    </LabSection>
  );
}

export function StatusSection() {
  return (
    <LabSection
      id="lab-status"
      index="05"
      title="Status"
      lead="Every state has a color, a shape and a word. Remove the color and the meaning survives."
    >
      <ul className="lab-status-grid">
        {STATUS_TONES.map((tone) => (
          <li key={tone} className="lab-status-grid__item">
            <StatusIndicator tone={tone} label={TONE_MEANING[tone].label} pill pulse={tone === "active"} />
            <StatusIndicator tone={tone} label={TONE_MEANING[tone].label} size="sm" />
            <span className="lab-status-grid__meta">
              <span className="lab-mono">
                {tone} · {STATUS_SHAPES[tone]}
              </span>
              <span className="lab-status-grid__meaning">{TONE_MEANING[tone].meaning}</span>
            </span>
          </li>
        ))}
      </ul>
    </LabSection>
  );
}

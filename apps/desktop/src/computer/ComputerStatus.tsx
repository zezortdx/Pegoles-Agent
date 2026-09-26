import type { CSSProperties } from "react";
import type { ComputerCommand, ComputerModel } from "../state/computerModel";
import type { HumanError } from "../state/errors";
import { wakeLevel } from "./MachineScreen";
import { PowerIcon } from "../ui/icons";

function BootSteps({ model }: { model: ComputerModel }) {
  return (
    <ol className="boot" aria-label="Startup">
      {model.steps.map((step) => (
        <li key={step.label} className="boot__step" data-state={step.state}>
          <span className="boot__mark pg-work-anim" aria-hidden="true" />
          {step.label}
          <span className="visually-hidden">{step.state === "done" ? ", done" : step.state === "active" ? ", in progress" : ""}</span>
        </li>
      ))}
    </ol>
  );
}

interface ComputerStatusProps {
  readonly model: ComputerModel;
  readonly busy: boolean;
  readonly error: HumanError | null;
  readonly onCommand: (command: ComputerCommand) => void;
  readonly onDismissError: () => void;
}

/**
 * No screen yet, drawn inside the screen's own frame: dark when off, its
 * glass brightening through the real boot stages, at most one action.
 */
export function ComputerStatus({ model, busy, error, onCommand, onDismissError }: ComputerStatusProps) {
  const headline = error ? error.title : model.headline;
  const body = error ? error.hint : model.body;
  const primary = error && model.primary ? { ...model.primary, label: "Try again" } : model.primary;
  const showPrimary = primary && primary.command !== "take" && primary.command !== "return";
  const dark = model.phase === "off" || model.phase === "needs-setup" || model.phase === "error" || model.phase === "unavailable" || !!error;
  return (
    <div className="machine-state" data-phase={model.phase} data-error={error ? true : undefined} style={{ "--wake": wakeLevel(model) } as CSSProperties}>
      <span className="machine-state__glow" aria-hidden="true" />
      {model.phase === "starting" && <span className="machine-state__scan pg-work-anim" aria-hidden="true" />}
      {dark && <span className="machine-state__power" aria-hidden="true"><PowerIcon size={18} /></span>}
      <p className="machine-state__headline" role={error ? "alert" : undefined}>{headline}</p>
      {body && <p className="machine-state__body">{body}</p>}
      {!error && model.devNote && <p className="machine-state__note">{model.devNote}</p>}
      {model.steps.length > 0 && !error && <BootSteps model={model} />}
      {(showPrimary || error) && (
        <div className="machine-state__actions">
          {showPrimary && (
            <button type="button" className={primary.command === "start" || primary.command === "install" ? "btn btn--primary" : "btn btn--line"} disabled={busy} onClick={() => onCommand(primary.command)}>
              {busy ? <><span className="spinner" aria-hidden="true" />Working…</> : primary.label}
            </button>
          )}
          {error && <button type="button" className="btn btn--quiet" onClick={onDismissError}>Dismiss</button>}
        </div>
      )}
    </div>
  );
}

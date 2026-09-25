import { PresenceMark } from "../presence";
import type { ComputerCommand, ComputerModel } from "../state/computerModel";
import { HandIcon, PauseIcon, RefreshIcon } from "../ui/icons";

export const RETURN_SHORTCUT = "⌃⌥⎋";

export interface ControlStripProps {
  readonly model: ComputerModel;
  readonly busy: boolean;
  readonly onCommand: (command: ComputerCommand) => void;
  /** Its live screen is here, so a person could take over. */
  readonly canTake: boolean;
  /** When the picture on screen was captured, if it is a snapshot. */
  readonly snapshotAge?: string | null;
  readonly onRefreshSnapshot?: () => void;
  readonly compact?: boolean;
}

function ownerLine(model: ComputerModel): string {
  if (model.phase === "paused") return "Paused";
  if (model.owner === "agent") return "Pegoles is using it";
  if (model.owner === "user") return "You have control";
  return "Nobody is using it";
}

/**
 * Who holds the computer, said plainly, and every real way to intervene in
 * plain view: take or give back control, pause, resume, stop. When taking
 * over isn't possible it says why instead of silently missing.
 */
export function ControlStrip({ model, busy, onCommand, canTake, snapshotAge, onRefreshSnapshot, compact = false }: ControlStripProps) {
  const handoff = model.primary && (model.primary.command === "take" || model.primary.command === "return") ? model.primary : undefined;
  const resume = model.phase === "paused" && model.primary?.command === "resume" ? model.primary : undefined;
  const pause = model.secondary.find((action) => action.command === "pause");
  const stop = model.secondary.find((action) => action.command === "stop");
  const why = !handoff && model.owner !== "user" && model.phase !== "paused" && !canTake
    ? "Taking over needs its live screen, which this build can’t show yet." : null;
  const mode = model.owner === "agent" ? "using-computer" : model.owner === "user" ? "waiting" : "idle";
  return (
    <div className="strip" data-owner={model.owner} data-phase={model.phase} data-compact={compact || undefined}>
      <div className="strip__row">
        <span className="strip__owner" role="status" aria-live="polite">
          <PresenceMark mode={mode} size={16} />
          <span>{ownerLine(model)}</span>
          {model.owner === "user" && <span className="strip__hint">Give it back with <kbd className="kbd">{RETURN_SHORTCUT}</kbd></span>}
        </span>
        {snapshotAge && (
          <span className="strip__snapshot">
            <span>Snapshot · {snapshotAge}</span>
            {onRefreshSnapshot && (
              <button type="button" className="icon-btn strip__refresh" aria-label="Refresh snapshot" title="Refresh snapshot" onClick={onRefreshSnapshot}>
                <RefreshIcon size={13} />
              </button>
            )}
          </span>
        )}
        <span className="strip__actions" role="group" aria-label="Computer controls">
          {resume && <button type="button" className="btn btn--small btn--primary" disabled={busy} onClick={() => onCommand(resume.command)}>{resume.label}</button>}
          {pause && <button type="button" className="btn btn--small btn--quiet" disabled={busy} onClick={() => onCommand(pause.command)}><PauseIcon size={12} />{pause.label}</button>}
          {stop && <button type="button" className="btn btn--small btn--quiet" disabled={busy} onClick={() => onCommand(stop.command)}>{stop.label}</button>}
          {handoff && (
            <button type="button" className={handoff.command === "take" ? "btn btn--small btn--line" : "btn btn--small btn--accent"} disabled={busy} onClick={() => onCommand(handoff.command)}>
              {handoff.command === "take" && <HandIcon size={13} />}{handoff.label}
            </button>
          )}
        </span>
      </div>
      {why && <p className="strip__why">{why}</p>}
    </div>
  );
}

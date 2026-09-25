import type { ComputerModel } from "../state/computerModel";
import { MachineScreen } from "../computer/MachineScreen";
import { AlertCircleIcon, ModelIcon, ShieldIcon } from "../ui/icons";

export interface ComposerStripProps {
  readonly computer: ComputerModel;
  readonly computerOpen: boolean;
  readonly onComputer: () => void;
}

/** Where the job runs: Pegoles' own computer, in its current state. Opens it. */
export function ComposerStrip({ computer, computerOpen, onComputer }: ComposerStripProps) {
  return (
    <>
      <button type="button" className="strip-item" data-phase={computer.phase} aria-pressed={computerOpen}
        aria-label={`Runs on Pegoles Computer: ${computer.chip}. ${computerOpen ? "Hide" : "Show"} its computer`} onClick={onComputer}>
        <MachineScreen model={computer} size="chip" />
        <span className="strip-item__label">Pegoles Computer</span>
        <span className="strip-item__state" data-phase={computer.phase}>{computer.chip}</span>
      </button>
      <span className="strip-note">Isolated from your Mac</span>
    </>
  );
}

export interface ComposerControlsProps {
  readonly modelReady: boolean;
  readonly modelName?: string;
  readonly onSafety: () => void;
  readonly onModel: () => void;
}

/** How the job runs: the fixed safety rules and the model, as facts you can open. */
export function ComposerControls({ modelReady, modelName, onSafety, onModel }: ComposerControlsProps) {
  return (
    <>
      <button type="button" className="composer-chip" onClick={onSafety} title="Pegoles asks before risky steps. See the rules.">
        <ShieldIcon size={14} />
        <span>Asks before risky steps</span>
      </button>
      <button type="button" className="composer-chip" data-tone={modelReady ? undefined : "attention"} onClick={onModel}
        aria-label={modelReady ? `Model: ${modelName ?? "connected"}` : "No model connected. Open settings"}>
        {modelReady ? <ModelIcon size={14} /> : <AlertCircleIcon size={14} />}
        <span>{modelReady ? modelName ?? "Model connected" : "No model"}</span>
      </button>
    </>
  );
}

import type { ComputerModel } from "../state/computerModel";
import { MachineScreen } from "../computer/MachineScreen";
import { AlertCircleIcon, ModelIcon, ShieldIcon } from "../ui/icons";
import { HOST } from "../lib/host";

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
      <span className="strip-note">Isolated from your {HOST}</span>
    </>
  );
}

export interface ComposerControlsProps {
  readonly modelReady: boolean;
  readonly modelName?: string;
  /** While not ready: what would make it ready ("Set up Pegoles Local"). Without it the chip says there is no model. */
  readonly setupLabel?: string;
  readonly onSafety: () => void;
  readonly onModel: () => void;
}

/** How the job runs: the fixed safety rules and the model, as facts you can open. */
export function ComposerControls({ modelReady, modelName, setupLabel, onSafety, onModel }: ComposerControlsProps) {
  const missing = setupLabel ? `${setupLabel.replace(/…$/, "")}. Open settings` : "No model connected. Open settings";
  return (
    <>
      <button type="button" className="composer-chip" onClick={onSafety} title="Pegoles acts only inside its own isolated computer. See the rules.">
        <ShieldIcon size={14} />
        <span>Stays inside its computer</span>
      </button>
      <button type="button" className="composer-chip" data-tone={modelReady ? undefined : "attention"} onClick={onModel}
        aria-label={modelReady ? `Model: ${modelName ?? "connected"}` : missing}>
        {modelReady ? <ModelIcon size={14} /> : <AlertCircleIcon size={14} />}
        <span>{modelReady ? modelName ?? "Model connected" : setupLabel ?? "No model"}</span>
      </button>
    </>
  );
}

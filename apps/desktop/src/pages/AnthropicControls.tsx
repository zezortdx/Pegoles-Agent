import { useId } from "react";
import type { ModelSettings } from "../lib/tauri";
import type { ModelOp, ModelSettingsState } from "../state/useModelSettings";
import { effortLabel, modelLabel } from "../lib/format";
import { ChevronDownIcon } from "../ui/icons";
import { Row } from "./settingsParts";

/** What choosing the cloud means for privacy. Pegoles Local sends nothing anywhere. */
export const PRIVACY_NOTE =
  "With Anthropic chosen, when a task runs, its text and screenshots of Pegoles’ computer (its own virtual machine, never your Mac’s screen) are sent to Anthropic’s API. The key stays in your Mac’s Keychain and never reaches Pegoles’ computer.";

const FAILED: Record<ModelOp, string> = {
  load: "Couldn’t read the Anthropic settings.",
  key: "Couldn’t save the key.",
  clear: "Couldn’t remove the key.",
  choice: "Couldn’t change the model.",
};

/** The cloud settings' failure as one sentence, in Core's words after ours. */
export function anthropicProblem(model: ModelSettingsState): string | null {
  const { error } = model;
  return error ? `${FAILED[error.op]} ${error.text.charAt(0).toUpperCase()}${error.text.slice(1)}` : null;
}

/** Where the key lives, in a few words. */
export function keyStatus(settings: ModelSettings | null): string {
  if (!settings) return "Checking…";
  if (settings.key_source === "keychain") return "Connected (Keychain)";
  if (settings.key_source === "environment") return "From ANTHROPIC_API_KEY";
  return "Not connected";
}

function Select({ id, label, value, options, disabled, onChange }: {
  id: string; label: string; value: string; options: readonly { value: string; label: string }[]; disabled: boolean; onChange: (value: string) => void;
}) {
  return (
    <span className="select">
      <select id={id} aria-label={label} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
      </select>
      <ChevronDownIcon size={12} className="select__chevron" />
    </span>
  );
}

export interface AnthropicControlsProps {
  readonly model: ModelSettingsState;
  /** A key was just stored (true), or anything else changed since (false). */
  readonly onSaved: (saved: boolean) => void;
}

/**
 * The cloud model's rows: whether a key is connected and where it lives,
 * a way to store one, and the model and effort to use. The key itself is
 * pasted into a macOS dialog Core opens, never into this page, and is never
 * shown again. Core is the authority; every value here is what it reports.
 */
export function AnthropicControls({ model, onSaved }: AnthropicControlsProps) {
  const { settings, pending } = model;
  const fieldId = useId();
  const busy = pending !== null;
  const configured = !!settings?.configured;

  const enter = async () => {
    if (busy) return;
    onSaved(false);
    if (await model.enterKey()) onSaved(true);
  };

  return (
    <>
      <Row label="Anthropic API key" hint={configured ? undefined : "Needed for tasks while Anthropic is chosen."}>
        <span className="setting__status">
          <span className="dot" data-tone={configured ? "done" : undefined} aria-hidden="true" />
          <span className={configured ? undefined : "setting__muted"}>{keyStatus(settings)}</span>
          {settings?.key_source === "keychain" && (
            <button type="button" className="btn btn--quiet btn--small" disabled={busy} onClick={() => { onSaved(false); void model.clearKey(); }}>
              {pending === "clear" ? "Removing…" : "Remove key"}
            </button>
          )}
        </span>
      </Row>
      <Row label={configured ? "Replace the key" : "Add a key"}
        hint="You paste it in a macOS window Pegoles opens, never on this screen. It’s kept in your Mac’s Keychain and isn’t shown again.">
        <button type="button" className="btn btn--line btn--small" disabled={busy} onClick={() => void enter()}>
          {pending === "key" ? "Waiting for the key…" : configured ? "Replace key…" : "Add key…"}
        </button>
      </Row>
      {settings && (
        <Row label="Model">
          <Select id={`${fieldId}-model`} label="Model" value={settings.model} disabled={busy}
            options={settings.models.map((value) => ({ value, label: modelLabel(value) }))}
            onChange={(value) => { onSaved(false); void model.choose(value, settings.effort); }} />
        </Row>
      )}
      {settings && (
        <Row label="Effort" hint="Higher effort thinks longer before each step: slower, and it costs more.">
          <Select id={`${fieldId}-effort`} label="Effort" value={settings.effort} disabled={busy}
            options={settings.efforts.map((value) => ({ value, label: effortLabel(value) }))}
            onChange={(value) => { onSaved(false); void model.choose(settings.model, value); }} />
        </Row>
      )}
    </>
  );
}

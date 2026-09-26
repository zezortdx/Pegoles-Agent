import { useId, useRef, useState, type FormEvent } from "react";
import type { ModelSettings } from "../lib/tauri";
import type { ModelOp, ModelSettingsState } from "../state/useModelSettings";
import { effortLabel, modelLabel } from "../lib/format";
import { ChevronDownIcon } from "../ui/icons";
import { Row, Section } from "./settingsParts";

export const PRIVACY_NOTE =
  "When a task runs, its text and screenshots of Pegoles’ computer (its own virtual machine, never your Mac’s screen) are sent to Anthropic’s API. The key stays in your Mac’s Keychain and never reaches Pegoles’ computer.";

const FAILED: Record<ModelOp, string> = {
  load: "Couldn’t read the model settings.",
  key: "Couldn’t save the key.",
  clear: "Couldn’t remove the key.",
  choice: "Couldn’t change the model.",
};

function keyStatus(settings: ModelSettings | null): string {
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

export interface ModelSectionProps {
  readonly id: string;
  readonly native: boolean;
  readonly model: ModelSettingsState;
}

/**
 * The model Pegoles works with: whether a key is connected and where it
 * lives, a way to store one (it is never shown again), and the model and
 * effort to use. Core is the authority; every value here is what it reports.
 */
export function ModelSection({ id, native, model }: ModelSectionProps) {
  const { settings, pending, error } = model;
  const keyRef = useRef<HTMLInputElement>(null);
  const fieldId = useId();
  const [filled, setFilled] = useState(false);
  const [saved, setSaved] = useState(false);
  const busy = pending !== null;
  const configured = !!settings?.configured;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const field = keyRef.current;
    if (!field || busy) return;
    setSaved(false);
    const ok = await model.saveKey(field.value);
    if (!ok) return;
    // Never kept, never shown again.
    field.value = "";
    setFilled(false);
    setSaved(true);
  };

  if (!native) {
    return (
      <Section id={id} title="Model" note={PRIVACY_NOTE}>
        <Row label="Anthropic API key"><span className="setting__muted">Desktop app required</span></Row>
      </Section>
    );
  }

  const problem = error ? `${FAILED[error.op]} ${error.text.charAt(0).toUpperCase()}${error.text.slice(1)}` : null;
  return (
    <Section
      id={id} title="Model" lead="Anthropic" note={PRIVACY_NOTE}
      after={(problem || saved) && (
        problem
          ? <p className="settings__problem" role="alert">{problem}</p>
          : <p className="settings__ok" role="status">Key saved to your Keychain.</p>
      )}
    >
      <Row label="Anthropic API key" hint={configured ? undefined : "Pegoles needs one to work on tasks."}>
        <span className="setting__status">
          <span className="dot" data-tone={configured ? "done" : undefined} aria-hidden="true" />
          <span className={configured ? undefined : "setting__muted"}>{keyStatus(settings)}</span>
          {settings?.key_source === "keychain" && (
            <button type="button" className="btn btn--quiet btn--small" disabled={busy} onClick={() => { setSaved(false); void model.clearKey(); }}>
              {pending === "clear" ? "Removing…" : "Remove key"}
            </button>
          )}
        </span>
      </Row>
      <form className="setting setting--form" role="listitem" aria-label="Save an API key" onSubmit={(event) => void submit(event)}>
        <label className="setting__text" htmlFor={fieldId}>
          <span className="setting__label">{configured ? "Replace the key" : "Add a key"}</span>
          <span className="setting__hint">Stored in your Mac’s Keychain. It isn’t shown again.</span>
        </label>
        <span className="setting__value setting__field">
          <input
            id={fieldId} ref={keyRef} className="field" type="password" placeholder="sk-ant-…"
            autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} disabled={busy}
            onInput={(event) => setFilled(event.currentTarget.value.trim().length > 0)}
          />
          <button type="submit" className="btn btn--line btn--small" disabled={!filled || busy}>
            {pending === "key" ? "Saving…" : "Save"}
          </button>
        </span>
      </form>
      {settings && (
        <Row label="Model">
          <Select id={`${fieldId}-model`} label="Model" value={settings.model} disabled={busy}
            options={settings.models.map((value) => ({ value, label: modelLabel(value) }))}
            onChange={(value) => { setSaved(false); void model.choose(value, settings.effort); }} />
        </Row>
      )}
      {settings && (
        <Row label="Effort" hint="Higher effort thinks longer before each step: slower, and it costs more.">
          <Select id={`${fieldId}-effort`} label="Effort" value={settings.effort} disabled={busy}
            options={settings.efforts.map((value) => ({ value, label: effortLabel(value) }))}
            onChange={(value) => { setSaved(false); void model.choose(settings.model, value); }} />
        </Row>
      )}
    </Section>
  );
}

import { useEffect, useId, useState, type ReactNode } from "react";
import type { Provider } from "../lib/tauri";
import type { ModelSettingsState } from "../state/useModelSettings";
import { intelligenceProblem, type IntelligenceState } from "../state/useIntelligence";
import { localView } from "../state/localModel";
import { AnthropicControls, anthropicProblem, PRIVACY_NOTE } from "./AnthropicControls";
import { LocalAdvanced, LocalStateRow, LocalStatus } from "./LocalModel";
import { Row } from "./settingsParts";

const LOCAL_HINT = "Free · Private · Runs on this Mac";
const ANTHROPIC_HINT = "Claude models, with your own API key";

interface ProviderRowProps {
  readonly name: string;
  readonly value: Provider;
  readonly checked: boolean;
  readonly disabled: boolean;
  readonly title: string;
  readonly tag?: string;
  readonly hint: string;
  readonly status: (id: string) => ReactNode;
  readonly onChoose: (value: Provider) => void;
}

/**
 * One way Pegoles can think, as a radio row. Both rows share a name, so
 * they are one group to the keyboard and to VoiceOver even though Local
 * and the cloud sit in different groups on screen.
 */
function ProviderRow({ name, value, checked, disabled, title, tag, hint, status, onChoose }: ProviderRowProps) {
  const id = useId();
  return (
    <label className="setting provider" role="listitem" data-checked={checked || undefined}>
      <input type="radio" className="radio" name={name} value={value} checked={checked} disabled={disabled}
        aria-labelledby={`${id}-title`} aria-describedby={`${id}-hint ${id}-status`} onChange={() => onChoose(value)} />
      <span className="setting__text">
        <span className="provider__title">
          <span id={`${id}-title`} className="setting__label">{title}</span>
          {tag && <span className="tag">{tag}</span>}
        </span>
        <span id={`${id}-hint`} className="setting__hint">{hint}</span>
      </span>
      <span className="setting__value">{status(`${id}-status`)}</span>
    </label>
  );
}

export interface IntelligenceSectionProps {
  readonly id: string;
  readonly native: boolean;
  readonly intelligence: IntelligenceState;
  /** The cloud provider's key, model and effort. */
  readonly model: ModelSettingsState;
}

/**
 * What Pegoles thinks with. Pegoles Local comes first and needs nothing
 * but a one-time download; the cloud is an option, never a requirement.
 * Every state and number here is what Core reports.
 */
export function IntelligenceSection({ id, native, intelligence, model }: IntelligenceSectionProps) {
  const [keySaved, setKeySaved] = useState(false);
  const name = useId();
  const { refresh } = intelligence;
  // What is loaded (and how much memory it uses) changes with every task: read it again on arrival.
  useEffect(() => { if (native) refresh(); }, [native, refresh]);

  const head = (
    <div className="settings__head">
      <h2 id={id} className="settings__title" tabIndex={-1}>Intelligence</h2>
      <p className="settings__lead">What Pegoles thinks with</p>
    </div>
  );

  if (!native) {
    return (
      <section className="settings__section" aria-labelledby={id} data-section={id}>
        {head}
        <div className="settings__group" role="list">
          <Row label="Pegoles Local" hint={LOCAL_HINT}><span className="setting__muted">Desktop app required</span></Row>
        </div>
        <h3 className="settings__subhead">Cloud (optional)</h3>
        <div className="settings__group" role="list">
          <Row label="Anthropic" hint={ANTHROPIC_HINT}><span className="setting__muted">Desktop app required</span></Row>
        </div>
      </section>
    );
  }

  const current = intelligence.intelligence;
  const view = localView(current);
  const provider = current?.provider ?? null;
  const locked = !current || intelligence.pending !== null;
  const cloud = model.settings ?? current?.anthropic ?? null;
  const problem = intelligenceProblem(intelligence.error);
  const cloudProblem = provider === "anthropic" ? anthropicProblem(model) : null;

  const choose = (next: Provider) => {
    if (next === provider) return;
    setKeySaved(false);
    void intelligence.setProvider(next);
  };
  const setUp = () => {
    if (!view.model) return;
    void (view.stage === "damaged" ? intelligence.repair(view.model.id) : intelligence.install(view.model.id));
  };

  return (
    <section className="settings__section intelligence" aria-labelledby={id} data-section={id}>
      {head}
      <div className="settings__group" role="list">
        <ProviderRow name={name} value="local" checked={provider === "local"} disabled={locked} onChoose={choose}
          title="Pegoles Local" tag="Recommended" hint={LOCAL_HINT} status={(statusId) => <LocalStatus id={statusId} view={view} />} />
        <LocalStateRow view={view} pending={intelligence.pending} onSetUp={setUp} onCancel={() => void intelligence.cancelInstall()} />
      </div>
      {problem && (
        <p className="settings__problem" role="alert">
          {problem}
          {intelligence.error?.op === "load" && (
            <> <button type="button" className="link-btn" onClick={refresh}>Try again</button></>
          )}
        </p>
      )}
      {current && (
        <LocalAdvanced intelligence={current} view={view} pending={intelligence.pending}
          onChooseModel={(next) => void intelligence.setProvider(current.provider, next)} onRemove={intelligence.remove} />
      )}

      <h3 className="settings__subhead">Cloud (optional)</h3>
      <div className="settings__group" role="list">
        <ProviderRow name={name} value="anthropic" checked={provider === "anthropic"} disabled={locked} onChoose={choose}
          title="Anthropic" hint={ANTHROPIC_HINT}
          status={(statusId) => <span id={statusId} className="setting__muted">{cloud?.configured ? "Key connected" : "No key"}</span>} />
        {provider === "anthropic" && <AnthropicControls model={model} onSaved={setKeySaved} />}
      </div>
      {cloudProblem
        ? <p className="settings__problem" role="alert">{cloudProblem}</p>
        : keySaved && provider === "anthropic" && <p className="settings__ok" role="status">Key saved to your Keychain.</p>}
      <p className="settings__note">{PRIVACY_NOTE}</p>
    </section>
  );
}

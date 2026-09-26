import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import type { Intelligence, LocalModelInfo } from "../lib/tauri";
import type { IntelligenceOp } from "../state/useIntelligence";
import { canSetUp, installActive, progressText, sentence, setupAction, type LocalStage, type LocalView } from "../state/localModel";
import { formatBytes, formatMemory, quantizationLabel } from "../lib/format";
import { LocalProgress } from "../intelligence/LocalProgress";
import { Row } from "./settingsParts";

const STATUS: Record<Exclude<LocalStage, "ready">, { readonly label: string; readonly tone?: "attention" | "error" }> = {
  checking: { label: "Checking…" },
  unsupported: { label: "Not available on this Mac" },
  "not-set-up": { label: "Not set up" },
  preparing: { label: "Setting up…" },
  paused: { label: "Paused" },
  failed: { label: "Didn’t finish", tone: "error" },
  damaged: { label: "Needs setting up again", tone: "attention" },
  downloaded: { label: "Can’t run yet", tone: "attention" },
};

/** Pegoles Local's state in a few words, beside its name. */
export function LocalStatus({ view, id }: { view: LocalView; id?: string }) {
  if (view.stage === "ready") {
    const text = !view.running ? "Ready" : view.footprintBytes ? `Running locally · ${formatMemory(view.footprintBytes)} in memory` : "Running locally";
    return <span id={id} className="setting__status"><span className="dot" data-tone="done" aria-hidden="true" />{text}</span>;
  }
  const status = STATUS[view.stage];
  return (
    <span id={id} className="setting__status" data-tone={status.tone}>
      {status.tone && <span className="dot" data-tone={status.tone} aria-hidden="true" />}
      <span className={status.tone ? "local-status__word" : "setting__muted"}>{status.label}</span>
    </span>
  );
}

export interface LocalStateRowProps {
  readonly view: LocalView;
  readonly pending: IntelligenceOp | null;
  readonly onSetUp: () => void;
  readonly onCancel: () => void;
}

/** What Pegoles Local needs, and the one thing to do about it. Nothing when it's ready. */
export function LocalStateRow({ view, pending, onSetUp, onCancel }: LocalStateRowProps) {
  if (view.stage === "checking" || view.stage === "ready") return null;
  if (view.stage === "preparing") {
    return (
      <div className="setting local-state" role="listitem">
        <LocalProgress view={view} />
        <button type="button" className="btn btn--quiet btn--small" disabled={pending !== null} onClick={onCancel}>
          {pending === "cancel" ? "Cancelling…" : "Cancel"}
        </button>
      </div>
    );
  }
  const size = view.model ? formatBytes(view.model.size_bytes) : null;
  let text: string;
  switch (view.stage) {
    case "unsupported": text = `${view.problem ?? "Pegoles Local needs a Mac with Apple silicon."} You can still use a cloud model.`; break;
    case "downloaded": text = `Downloaded, but it can’t run here yet. ${view.problem ?? ""}`.trim(); break;
    case "paused": text = `Paused at ${progressText(view.doneBytes ?? 0, view.totalBytes ?? 0)}. It picks up where it stopped.`; break;
    case "failed": text = sentence(view.error ?? "The setup didn’t finish."); break;
    case "damaged": text = `Its files didn’t pass the check${view.reason ? ` (${view.reason})` : ""}. Setting it up again replaces them.`; break;
    default: text = size ? `One download of ${size}, checked before it’s used. After that it works offline.` : "One download, checked before it’s used. After that it works offline.";
  }
  const problem = view.stage === "not-set-up" || view.stage === "paused" || view.stage === "failed" ? view.problem : null;
  return (
    <div className="setting local-state" role="listitem">
      <div className="local-state__text">
        <p data-tone={view.stage === "failed" ? "error" : undefined} role={view.stage === "failed" ? "alert" : undefined}>{text}</p>
        {problem && <p className="local-state__problem">{problem}</p>}
      </div>
      {canSetUp(view) && (
        <button type="button" className="btn btn--primary btn--small" disabled={pending !== null} onClick={onSetUp}>
          {pending === "install" ? "Starting…" : setupAction(view)}
        </button>
      )}
    </div>
  );
}

function modelWord(model: LocalModelInfo, intelligence: Intelligence): string {
  const job = intelligence.local.install;
  if (job?.model === model.id && installActive(job)) return "Setting up…";
  switch (model.state) {
    case "installed": return intelligence.local.loaded_model === model.id ? "Running locally" : "Downloaded";
    case "partial": return model.partial_bytes ? `Paused at ${formatBytes(model.partial_bytes)}` : "Paused";
    case "invalid": return "Needs setting up again";
    default: return "Not downloaded";
  }
}

/** Every model this build can run, to choose which one Pegoles Local uses. */
function ModelChoice({ intelligence, chosen, disabled, onChoose }: {
  intelligence: Intelligence; chosen: string; disabled: boolean; onChoose: (id: string) => void;
}) {
  const labelId = useId();
  const name = useId();
  return (
    <>
      <p id={labelId} className="advanced__label">Which model Pegoles Local uses</p>
      <div className="settings__group" role="radiogroup" aria-labelledby={labelId}>
        {intelligence.local.models.map((model) => (
          <label key={model.id} className="setting provider provider--compact" data-checked={model.id === chosen || undefined}>
            <input type="radio" className="radio" name={name} value={model.id} checked={model.id === chosen} disabled={disabled}
              aria-label={`${model.display_name}, ${quantizationLabel(model.quantization)}`} onChange={() => onChoose(model.id)} />
            <span className="setting__text">
              <span className="provider__title">
                <span className="setting__label">{model.display_name}</span>
                {model.id === intelligence.local.default_model && <span className="tag">Default</span>}
              </span>
              <span className="setting__hint">{quantizationLabel(model.quantization)} · {formatBytes(model.size_bytes)}</span>
            </span>
            <span className="setting__value setting__muted">{modelWord(model, intelligence)}</span>
          </label>
        ))}
      </div>
    </>
  );
}

/** Deleting a model's bytes never happens in one click: it asks inline first, with Cancel focused. */
function RemoveModel({ model, disabled, onRemove }: { model: LocalModelInfo; disabled: boolean; onRemove: (id: string) => Promise<boolean> }) {
  const [asking, setAsking] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const returning = useRef(false);
  const titleId = useId();
  const partial = model.state === "partial";

  useEffect(() => {
    if (asking) { cancelRef.current?.focus(); return; }
    if (returning.current) triggerRef.current?.focus();
    returning.current = false;
  }, [asking]);

  const dismiss = () => { returning.current = true; setAsking(false); };
  if (asking) {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      dismiss();
    };
    return (
      <div className="manage manage--confirm local-remove" role="group" aria-labelledby={titleId} onKeyDown={onKeyDown}>
        <p id={titleId} className="manage__question">{partial ? "Discard the partial download?" : `Remove ${model.display_name}?`}</p>
        <p className="manage__body">
          {partial ? "What was downloaded so far is deleted." : `Its ${formatBytes(model.size_bytes)} are deleted from this Mac.`} Tasks can’t use Pegoles Local until you set it up again.
        </p>
        <div className="manage__actions">
          <button ref={cancelRef} type="button" className="btn btn--quiet btn--small" onClick={dismiss}>Cancel</button>
          <button type="button" className="btn btn--danger btn--small" disabled={disabled}
            onClick={() => { void onRemove(model.id).then((ok) => { if (!ok) dismiss(); }); }}>
            {partial ? "Discard" : "Remove"}
          </button>
        </div>
      </div>
    );
  }
  return (
    <button ref={triggerRef} type="button" className="btn btn--quiet btn--small local-remove__trigger" disabled={disabled} onClick={() => setAsking(true)}>
      {partial ? "Discard download…" : "Remove model…"}
    </button>
  );
}

function macLabel(intelligence: Intelligence): string {
  const { chip, memory_bytes, apple_silicon } = intelligence.local;
  const name = chip ?? (apple_silicon ? "Apple silicon" : "Not Apple silicon");
  return memory_bytes > 0 ? `${name} · ${formatMemory(memory_bytes)} memory` : name;
}

export interface LocalAdvancedProps {
  readonly intelligence: Intelligence;
  readonly view: LocalView;
  readonly pending: IntelligenceOp | null;
  readonly onChooseModel: (id: string) => void;
  readonly onRemove: (id: string) => Promise<boolean>;
}

/**
 * The details a curious person may want, kept out of everyone else's way:
 * exactly what runs, where it came from and its license, all as Core's
 * catalog reports them.
 */
export function LocalAdvanced({ intelligence, view, pending, onChooseModel, onRemove }: LocalAdvancedProps) {
  const { model } = view;
  if (!model) return null;
  const setting = installActive(intelligence.local.install);
  const removable = model.state === "installed" || model.state === "invalid" || model.state === "partial";
  return (
    <details className="details advanced">
      <summary>Advanced</summary>
      <div className="advanced__body">
        <div className="settings__group settings__group--dense" role="list" aria-label="Pegoles Local model">
          <Row label="Model"><span>{model.display_name}</span></Row>
          <Row label="Parameters"><span className="mono">{model.parameters}</span></Row>
          <Row label="Quantization"><span className="mono">{quantizationLabel(model.quantization)}</span></Row>
          <Row label={model.state === "installed" ? "Size on disk" : "Download size"}><span className="mono">{formatBytes(model.size_bytes)}</span></Row>
          <Row label="License"><span className="mono">{model.license}</span></Row>
          <Row label="Source" wrap><span className="mono selectable">{model.source}</span></Row>
          <Row label="This Mac"><span>{macLabel(intelligence)}</span></Row>
          {model.recommended_min_ram_gb !== null && (
            <Row label="Recommended memory"><span>{model.recommended_min_ram_gb} GB or more</span></Row>
          )}
        </div>
        {intelligence.local.models.length > 1 && (
          <ModelChoice intelligence={intelligence} chosen={model.id} disabled={pending !== null || setting} onChoose={onChooseModel} />
        )}
        {removable && <RemoveModel model={model} disabled={pending !== null || setting} onRemove={onRemove} />}
      </div>
    </details>
  );
}

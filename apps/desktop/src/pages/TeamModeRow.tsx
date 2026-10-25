import { t, useLocale } from "../i18n";
import { useId } from "react";

export interface TeamModeRowProps {
  /** Core says team mode would run a task now. */
  readonly enabled: boolean;
  /** A cloud model is chosen: team mode never exists on Pegoles Local. */
  readonly available: boolean;
  /** Settings are loading or another change is in flight. */
  readonly busy: boolean;
  /** This switch is waiting for the person's answer in the macOS dialog. */
  readonly waiting: boolean;
  readonly onChange: (enabled: boolean) => void;
}

/**
 * The opt-in for experimental team mode. The switch only asks: turning it
 * on opens a macOS dialog that spells out what is sent to the cloud, and
 * what Core reports afterwards is what this shows. Off by default.
 */
export function TeamModeRow({ enabled, available, busy, waiting, onChange }: TeamModeRowProps) {
  useLocale();
  const id = useId();
  const state = available ? undefined : t("Needs a cloud model");
  return (
    <div className="setting" role="listitem" data-checked={enabled || undefined}>
      <span className="setting__text">
        <span className="provider__title">
          <label id={`${id}-title`} htmlFor={`${id}-switch`} className="setting__label">{t("Team mode")}</label>
          <span className="tag">{t("Experimental")}</span>
        </span>
        <span id={`${id}-hint`} className="setting__hint">
          {t("A captain, researcher, executor and critic take each task in turn. Slower, and several cloud requests per task. Only the executor acts; every action is still checked by the safety policy.")}
        </span>
        <span id={`${id}-note`} className="setting__hint">
          {t("Team mode sends the task text, screenshots of Pegoles’ computer and the team’s own notes to Anthropic, in several requests per task. It stays off until you confirm it in a macOS dialog, and never runs on Pegoles Local.")}
        </span>
      </span>
      <span className="setting__value">
        {state && <span className="setting__muted">{state}</span>}
        <input id={`${id}-switch`} type="checkbox" role="switch" className="radio" checked={enabled && available}
          disabled={!available || busy} aria-describedby={`${id}-hint ${id}-note${waiting ? ` ${id}-wait` : ""}`}
          onChange={(event) => onChange(event.target.checked)} />
        {waiting && <span id={`${id}-wait`} role="status" className="setting__muted">{t("Waiting for confirmation in the macOS dialog…")}</span>}
      </span>
    </div>
  );
}

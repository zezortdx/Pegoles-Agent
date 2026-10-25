import { t, useLocale } from "../i18n";
import { useId, useState } from "react";
import { PresenceAvatar } from "../presence";
import { modeFor, TEAM_ROLES, type TeamRun } from "../state/teamRun";
import { ChevronRightIcon } from "../ui/icons";
import { ROLE_NAME, ROLE_PROFILE, STATE_WORD, STEP_TITLE } from "./roles";
import "./team.css";

/** One line for the live region: who is working, or how the team ended. */
function headline(run: TeamRun): string {
  if (run.active) return t("{role}: {status}", { role: t(STEP_TITLE[run.active]), status: t("Working") });
  if (!run.settled) return t("The team is working…");
  return run.failed ? t("The team stopped before finishing.") : t("The team finished.");
}

/** The four roles as avatars: shape, name and state in words. Real states only. */
export function TeamStrip({ run }: { readonly run: TeamRun }) {
  useLocale();
  return (
    <ul className="team-strip" role="list" aria-label={t("Team")}>
      {TEAM_ROLES.map((role) => {
        const state = run.roles[role];
        const name = t(ROLE_NAME[role]);
        return (
          <li key={role} className="team-role" data-role={role} data-state={state}>
            <PresenceAvatar profile={{ ...ROLE_PROFILE[role], name }} mode={modeFor(state)} size={44} field={false} decorative />
            <span className="team-role__name">{name}</span>
            <span className="team-role__state">{t(STATE_WORD[state])}</span>
          </li>
        );
      })}
    </ul>
  );
}

export interface TeamPanelProps {
  readonly run: TeamRun;
  /** The team's final answer (Core's summary), shown as the prominent result. Untrusted text. */
  readonly result?: string | null;
}

/**
 * Experimental team mode on a task: the strip, the final result, and a
 * collapsible account of what each role reported. Everything shown is a
 * bounded status line or the final answer, as plain text; hidden reasoning
 * is never requested and never shown.
 */
export function TeamPanel({ run, result }: TeamPanelProps) {
  useLocale();
  const [open, setOpen] = useState(false);
  const heading = useId();
  const listId = useId();
  return (
    <section className="team" aria-labelledby={heading} data-failed={run.failed || undefined}>
      <h2 id={heading} className="team__title">{t("Team")} <span className="tag">{t("Experimental")}</span></h2>
      <p className="team__status" role="status">{headline(run)}</p>
      <TeamStrip run={run} />
      {result && (
        <section className="team-result" aria-label={t("Team result")}>
          <h3 className="team-result__title">{t("Team result")}</h3>
          <p className="team-result__text">{result}</p>
        </section>
      )}
      <button type="button" className="team__toggle" aria-expanded={open} aria-controls={listId} onClick={() => setOpen((value) => !value)}>
        <ChevronRightIcon size={12} className="team__chevron" />
        {t("How the team thought")}
      </button>
      <div id={listId} className="team__steps" hidden={!open}>
        <p className="team__note">{t("Short status notes from each role. Hidden reasoning is never shown.")}</p>
        <ol className="team-steps">
          {run.steps.map((step) => (
            <li key={step.step} className="team-step" data-state={step.state}>
              <span className="team-step__head">
                <span className="team-step__title">{t(STEP_TITLE[step.step])}</span>
                <span className="team-step__state">{t(STATE_WORD[step.state])}</span>
              </span>
              {step.summary && <p className="team-step__summary">{step.summary}</p>}
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

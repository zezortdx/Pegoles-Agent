import { useEffect, useId, useState } from "react";
import { api, type FixOutcome, type SystemCheck } from "../../lib/tauri";
import { errorText } from "../../state/errors";
import { RefreshIcon, ShieldIcon } from "../../ui/icons";
import { checkVerdict, technicalReport, TONE_WORD, type CheckRow } from "../systemCheck";
import { ToneGlyph } from "../visuals";
import { TechnicalDetails } from "./details";
import { DiagnosticReportButton } from "../../ui/DiagnosticReport";
import type { ScreenProps } from "./types";

/** Rows resolve one by one; this far apart. */
const REVEAL_MS = 170;
const ROW_COUNT = 6;

export interface CheckScreenProps extends ScreenProps {
  readonly check: SystemCheck | null;
  readonly checking: boolean;
  /** Core could not run the check at all. */
  readonly failure: string | null;
  readonly onRecheck: () => void;
  /** Resumed after the restart the person agreed to. */
  readonly resumedAfterRestart: boolean;
}

type FixState =
  | { readonly kind: "idle" }
  | { readonly kind: "asking" }
  | { readonly kind: "working" }
  | { readonly kind: "declined" }
  | { readonly kind: "restart" }
  | { readonly kind: "restarting" }
  | { readonly kind: "failed"; readonly technical: string };

/** Screen 3: can this computer run Pegoles? Friendly rows, and the fix when there is one. */
export function CheckScreen(props: CheckScreenProps) {
  const { headingRef, go, computer, check, checking, failure, onRecheck, animated, resumedAfterRestart } = props;
  const verdict = check ? checkVerdict(check) : null;
  const [revealed, setRevealed] = useState(animated ? 0 : ROW_COUNT);
  const [fix, setFix] = useState<FixState>({ kind: "idle" });

  // Reveal rows one at a time once the facts are in (instantly under Reduce Motion).
  useEffect(() => {
    if (!check || !animated) { setRevealed(animated ? 0 : ROW_COUNT); return; }
    setRevealed(0);
    const timer = window.setInterval(() => setRevealed((n) => (n >= ROW_COUNT ? n : n + 1)), REVEAL_MS);
    return () => window.clearInterval(timer);
  }, [check, animated]);

  const allShown = !!verdict && revealed >= verdict.rows.length;

  const turnOn = async () => {
    setFix({ kind: "working" });
    try {
      const outcome: FixOutcome = await api.fixVirtualization();
      if (outcome === "declined") setFix({ kind: "declined" });
      else if (outcome === "restart_required") { setFix({ kind: "restart" }); onRecheck(); }
      else { setFix({ kind: "idle" }); onRecheck(); }
    } catch (error) {
      setFix({ kind: "failed", technical: errorText(error) });
    }
  };
  const restart = async () => {
    setFix({ kind: "restarting" });
    try { await api.restartToFinishSetup(); } catch (error) { setFix({ kind: "failed", technical: errorText(error) }); }
  };

  const restartNeeded = fix.kind === "restart" || fix.kind === "restarting" || verdict?.fix?.kind === "restart";
  return (
    <section className="ob-screen" aria-labelledby="ob-title" aria-busy={checking || undefined}>
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title">Checking this {computer}</h1>
      <p className="ob-lead">
        {resumedAfterRestart ? "Welcome back. Let’s check again now that Windows has restarted." : `Pegoles needs a few things to run its own computer. This only takes a moment.`}
      </p>

      {failure ? (
        <div className="ob-note" data-tone="blocked" role="alert">
          <p className="ob-note__title">Pegoles couldn’t check this {computer}.</p>
          <p>Try again. If it keeps happening, restart Pegoles.</p>
          <TechnicalDetails text={failure} />
          <DiagnosticReportButton />
        </div>
      ) : (
        <ul className="ob-checks" aria-label="System check" aria-live="polite">
          {verdict
            ? verdict.rows.map((row, index) => <CheckItem key={row.id} row={row} shown={index < revealed} />)
            : Array.from({ length: ROW_COUNT }, (_, i) => <PendingItem key={i} />)}
        </ul>
      )}

      {allShown && verdict?.fix?.kind === "enable-virtualization" && !restartNeeded && (
        <FixPanel fix={fix} computer={computer} onAsk={() => setFix({ kind: "asking" })} onConfirm={() => void turnOn()} onCancel={() => setFix({ kind: "idle" })} />
      )}
      {allShown && restartNeeded && (
        <div className="ob-panel" role="region" aria-labelledby="ob-restart-title">
          <h2 id="ob-restart-title" className="ob-panel__title">One restart needed</h2>
          <p>Windows needs to restart once to finish turning on the isolated computer Pegoles uses. Save your work first.</p>
          <p className="ob-muted">After you sign back in, open Pegoles and it continues from here.</p>
          <div className="ob-actions ob-actions--inline">
            <button type="button" className="btn btn--quiet" disabled={fix.kind === "restarting"} onClick={() => setFix({ kind: "idle" })}>Later</button>
            <button type="button" className="btn btn--primary" disabled={fix.kind === "restarting"} onClick={() => void restart()}>
              {fix.kind === "restarting" ? "Restarting…" : "Restart now"}
            </button>
          </div>
        </div>
      )}
      {allShown && verdict?.fix?.kind === "firmware" && <FirmwareHelp />}
      {fix.kind === "failed" && (
        <div className="ob-note" data-tone="blocked" role="alert">
          <p className="ob-note__title">Pegoles couldn’t turn virtualization on.</p>
          <p>Nothing else was changed. Try again, or restart your {computer} and check again.</p>
          <TechnicalDetails text={fix.technical} />
          <DiagnosticReportButton />
        </div>
      )}

      {allShown && check && verdict && <TechnicalDetails text={technicalReport(check, verdict)} label="Technical details" />}
      {allShown && verdict && !verdict.canContinue && <DiagnosticReportButton className="ob-report" />}

      <div className="ob-actions">
        <button type="button" className="btn btn--quiet" onClick={() => go("how")}>Back</button>
        <span className="ob-actions__spacer" />
        {(!verdict?.canContinue || failure) && (
          <button type="button" className="btn btn--line" disabled={checking} onClick={onRecheck}>
            <RefreshIcon size={14} />{checking ? "Checking…" : "Check again"}
          </button>
        )}
        {verdict?.canContinue && (
          <button type="button" className="btn btn--primary ob-cta" disabled={!allShown} onClick={() => go("setup")}>Continue</button>
        )}
      </div>
    </section>
  );
}

function CheckItem({ row, shown }: { readonly row: CheckRow; readonly shown: boolean }) {
  if (!shown) return <PendingItem />;
  return (
    <li className="ob-check" data-tone={row.tone}>
      <ToneGlyph tone={row.tone} />
      <span className="ob-check__text">
        <span className="ob-check__title">{row.title}</span>
        {row.detail && <span className="ob-check__detail">{row.detail}</span>}
      </span>
      <span className="visually-hidden">{TONE_WORD[row.tone]}</span>
    </li>
  );
}

function PendingItem() {
  return (
    <li className="ob-check ob-check--pending" aria-hidden="true">
      <ToneGlyph tone="pending" />
      <span className="ob-check__text"><span className="ob-skeleton" /></span>
    </li>
  );
}

function FixPanel({ fix, computer, onAsk, onConfirm, onCancel }: {
  readonly fix: FixState; readonly computer: string; readonly onAsk: () => void; readonly onConfirm: () => void; readonly onCancel: () => void;
}) {
  const asking = fix.kind === "asking" || fix.kind === "working";
  return (
    <div className="ob-panel" role="region" aria-labelledby="ob-fix-title">
      <h2 id="ob-fix-title" className="ob-panel__title">Turn on virtualization</h2>
      {!asking ? (
        <>
          <p>Pegoles can turn on the Windows feature it needs. It takes a minute, and Windows usually asks to restart afterwards.</p>
          {fix.kind === "declined" && <p className="ob-muted" role="status">Nothing was changed. You can do this whenever you’re ready.</p>}
          <div className="ob-actions ob-actions--inline">
            <button type="button" className="btn btn--primary" onClick={onAsk}>Fix automatically</button>
          </div>
        </>
      ) : (
        <>
          <p className="ob-panel__why">
            <ShieldIcon size={14} />
            <span>Windows will now ask for administrator permission. Pegoles only turns on <strong>Virtual Machine Platform</strong>, a built-in part of Windows. Nothing else on this {computer} changes, and nothing is downloaded.</span>
          </p>
          <div className="ob-actions ob-actions--inline">
            <button type="button" className="btn btn--quiet" disabled={fix.kind === "working"} onClick={onCancel}>Not now</button>
            <button type="button" className="btn btn--primary" disabled={fix.kind === "working"} onClick={onConfirm}>
              {fix.kind === "working" ? "Waiting for Windows…" : "Continue"}
            </button>
          </div>
        </>
      )}
    </div>
  );
}

function FirmwareHelp() {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className="ob-panel" role="region" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="ob-panel__title">Turn it on in the firmware settings</h2>
      <p>It takes a few minutes and a restart. Pegoles can’t change firmware (BIOS/UEFI) settings itself, but the steps are short.</p>
      <button type="button" className="btn btn--line ob-fit" aria-expanded={open} aria-controls={id} onClick={() => setOpen((value) => !value)}>
        {open ? "Hide the steps" : "Show me how"}
      </button>
      {open && (
        <ol id={id} className="ob-steps">
          <li>Save your work, then restart the PC.</li>
          <li>While it starts, press the firmware key shown on screen (often <kbd className="kbd">F2</kbd>, <kbd className="kbd">Del</kbd>, <kbd className="kbd">F10</kbd> or <kbd className="kbd">Esc</kbd>).</li>
          <li>Find the setting called <strong>Intel Virtualization Technology</strong> (VT-x) or <strong>SVM Mode</strong> (AMD). It’s often under Advanced or CPU Configuration.</li>
          <li>Set it to <strong>Enabled</strong>, then save and exit.</li>
          <li>When Windows is back, open Pegoles and choose <strong>Check again</strong>.</li>
        </ol>
      )}
    </div>
  );
}

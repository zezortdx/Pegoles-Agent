import type { StatusPayload } from "../../lib/tauri";
import { formatBytes } from "../../lib/format";
import type { LocalView } from "../../state/localModel";
import { CheckIcon, PauseIcon } from "../../ui/icons";
import { ProgressMeter } from "../../ui/ProgressMeter";
import { errorHint, type SetupProgress } from "../setupProgress";
import { formatRate, formatTimeLeft, useTransferRate } from "../transferRate";
import type { SetupRun } from "../useSetupRun";
import { MachineGlyph } from "../visuals";
import { TechnicalDetails } from "./details";
import type { ScreenProps } from "./types";

export interface SetupScreenProps extends ScreenProps {
  readonly run: SetupRun;
  readonly local: LocalView;
  readonly status: StatusPayload | null;
}

/** Screen 4: one button, then honest progress for everything Pegoles needs. */
export function SetupScreen({ headingRef, go, computer, run, local, status }: SetupScreenProps) {
  const { progress } = run;
  const rate = useTransferRate(progress.doneBytes, progress.totalBytes, progress.phase === "working" && progress.transferring);
  const modelSize = local.model?.size_bytes ?? 0;
  const imageSize = status?.image_setup?.download_bytes ?? 0;
  const machineState = progress.phase === "done" ? "done" : progress.phase === "failed" ? "failed" : progress.phase === "working" ? "working" : "idle";

  return (
    <section className="ob-screen ob-screen--setup" aria-labelledby="ob-title">
      <MachineGlyph state={machineState} />
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title">
        {progress.phase === "done" ? "Pegoles is set up" : progress.phase === "idle" ? "Set up Pegoles" : progress.headline}
      </h1>

      {progress.phase === "idle" && (
        <>
          <p className="ob-lead">Pegoles downloads its AI model and prepares its own computer. This happens once; after that it works offline.</p>
          <ul className="ob-summary" aria-label="What will be set up">
            {progress.steps.some((s) => s.id === "model") && <li><span>AI model</span><span className="ob-summary__size">{modelSize ? formatBytes(modelSize) : "—"}</span></li>}
            {progress.steps.some((s) => s.id === "computer") && <li><span>Pegoles’ computer</span><span className="ob-summary__size">{imageSize ? formatBytes(imageSize) : "—"}</span></li>}
            <li><span>Total download</span><span className="ob-summary__size">{formatBytes(progress.totalBytes)}</span></li>
          </ul>
          {progress.resumable && <p className="ob-muted">{formatBytes(progress.doneBytes)} is already downloaded. Setup continues from there.</p>}
          <div className="ob-actions">
            <button type="button" className="btn btn--quiet" onClick={() => go("check")}>Back</button>
            <span className="ob-actions__spacer" />
            <button type="button" className="btn btn--primary ob-cta" onClick={run.start}>{progress.resumable ? "Continue setup" : "Set up Pegoles"}</button>
          </div>
        </>
      )}

      {(progress.phase === "working" || progress.phase === "paused") && (
        <>
          <p className="ob-lead" aria-live="polite">
            {progress.phase === "paused" ? "Paused. What was downloaded is kept; resume whenever you like." : "You can leave this window open and do something else."}
          </p>
          <Progress progress={progress} rate={rate} />
          <StepList progress={progress} />
          <div className="ob-actions">
            <span className="ob-actions__spacer" />
            {progress.phase === "working"
              ? <button type="button" className="btn btn--line" onClick={run.pause} disabled={!progress.transferring}><PauseIcon size={14} />Pause</button>
              : <button type="button" className="btn btn--primary ob-cta" onClick={run.start}>Resume</button>}
          </div>
        </>
      )}

      {progress.phase === "failed" && progress.error && (
        <>
          <div className="ob-note" data-tone="blocked" role="alert">
            <p className="ob-note__title">Something went wrong while setting up Pegoles</p>
            <p>{errorHint(progress.error.kind)}</p>
            <TechnicalDetails text={progress.error.technical} />
          </div>
          <StepList progress={progress} />
          <div className="ob-actions">
            <span className="ob-actions__spacer" />
            <button type="button" className="btn btn--primary ob-cta" onClick={run.retry}>Try again</button>
          </div>
        </>
      )}

      {progress.phase === "done" && (
        <>
          <p className="ob-lead">Its AI model is installed and its computer is running, isolated from your {computer}.</p>
          <StepList progress={progress} />
          <div className="ob-actions">
            <span className="ob-actions__spacer" />
            <button type="button" className="btn btn--primary ob-cta" onClick={() => go("intelligence")}>Continue</button>
          </div>
        </>
      )}
    </section>
  );
}

function Progress({ progress, rate }: { readonly progress: SetupProgress; readonly rate: ReturnType<typeof useTransferRate> }) {
  const measured = progress.totalBytes > 0 && progress.percent !== null;
  const bytes = `${formatBytes(progress.doneBytes)} of ${formatBytes(progress.totalBytes)}`;
  return (
    <div className="ob-progress">
      <div className="ob-progress__line">
        <span className="ob-progress__bytes">{measured ? bytes : "Getting ready…"}</span>
        {measured && <span className="ob-progress__percent">{progress.percent}%</span>}
      </div>
      <ProgressMeter
        label="Setting up Pegoles"
        percent={measured ? progress.percent : null}
        valueText={measured ? `${bytes}, ${progress.headline}` : progress.headline}
        working={progress.phase === "working" && !progress.transferring}
      />
      <div className="ob-progress__rate" aria-hidden={!rate || undefined}>
        {rate && progress.phase === "working" ? <>{formatRate(rate.bytesPerSecond)}{rate.secondsLeft !== null && <> · {formatTimeLeft(rate.secondsLeft)}</>}</> : " "}
      </div>
    </div>
  );
}

function StepList({ progress }: { readonly progress: SetupProgress }) {
  return (
    <ol className="ob-steplist" aria-label="Setup steps">
      {progress.steps.map((step) => (
        <li key={step.id} className="ob-steplist__item" data-state={step.state}>
          <span className="ob-steplist__dot" aria-hidden="true">{step.state === "done" ? <CheckIcon size={12} /> : null}</span>
          <span>{step.label}</span>
          <span className="visually-hidden">{step.state === "done" ? "done" : step.state === "active" ? "in progress" : "waiting"}</span>
        </li>
      ))}
    </ol>
  );
}

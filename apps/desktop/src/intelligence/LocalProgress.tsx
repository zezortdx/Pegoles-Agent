import { percentOf, progressText, STEP_LABEL, type LocalView } from "../state/localModel";
import { ProgressMeter } from "../ui/ProgressMeter";

/**
 * Pegoles Local being set up: what step it is on, the bytes Core counted
 * and a meter of them. Shared by Settings and a task waiting for it, so
 * both say exactly the same thing. Only the step is announced; bytes
 * change four times a second and are there to glance at.
 */
export function LocalProgress({ view }: { view: LocalView }) {
  const step = view.step ?? "downloading";
  const total = view.totalBytes ?? view.model?.size_bytes ?? 0;
  const done = Math.min(view.doneBytes ?? 0, total);
  const percent = percentOf(done, total);
  const measured = step === "downloading" && total > 0;
  return (
    <div className="local-progress" data-step={step}>
      <div className="local-progress__line">
        <span className="local-progress__step" aria-live="polite">{STEP_LABEL[step]}</span>
        {measured && <span className="local-progress__bytes">{progressText(done, total)}</span>}
        {measured && percent !== null && <span className="local-progress__percent">{percent}%</span>}
      </div>
      <ProgressMeter
        label="Setting up Pegoles Local"
        percent={total > 0 ? percent : null}
        valueText={measured ? progressText(done, total) : STEP_LABEL[step]}
        working={step !== "downloading"}
      />
    </div>
  );
}

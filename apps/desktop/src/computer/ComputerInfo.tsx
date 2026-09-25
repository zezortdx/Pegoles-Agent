import { useState } from "react";
import type { ComputerModel } from "../state/computerModel";
import type { HumanError } from "../state/errors";
import type { BootLogPayload, StatusPayload } from "../lib/tauri";

/** Plain facts Core reports about the machine. */
export function Facts({ model }: { model: ComputerModel }) {
  if (!model.facts.length) return null;
  return (
    <section className="computer__section" aria-labelledby="computer-facts">
      <h3 id="computer-facts" className="computer__section-title">This computer</h3>
      <dl className="facts">
        {model.facts.map((fact) => (
          <div key={fact.label} className="facts__row"><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
        ))}
      </dl>
    </section>
  );
}

interface DetailsProps {
  readonly model: ComputerModel;
  readonly status: StatusPayload | null;
  readonly error: HumanError | null;
  readonly loadBootLog: () => Promise<BootLogPayload>;
}

/** What engineers need, closed by default. Raw errors live only here. */
export function Details({ model, status, error, loadBootLog }: DetailsProps) {
  const [log, setLog] = useState<BootLogPayload | null>(null);
  const [logError, setLogError] = useState<string | null>(null);
  const issues = [error?.detail, status?.display_error, status?.display_setup_error, status?.viewport_issue].filter((value): value is string => !!value);
  const canReadLog = !!status?.computer_created && status.backend === "real";
  if (!model.specs.length && !issues.length) return null;
  return (
    <details className="details computer__details" onToggle={(event) => {
      if (!event.currentTarget.open || !canReadLog || log) return;
      loadBootLog().then(setLog).catch((reason: unknown) => setLogError(String(reason)));
    }}>
      <summary>Details</summary>
      {model.specs.length > 0 && (
        <dl className="facts facts--details">
          <div className="facts__row"><dt>Configuration</dt><dd>{model.specs.join(" · ")}</dd></div>
        </dl>
      )}
      {issues.length > 0 && <pre className="diagnostic">{issues.join("\n")}</pre>}
      {canReadLog && (log?.available ? <pre className="diagnostic">{log.tail.join("\n")}</pre>
        : <p className="computer__muted">{logError ?? (log ? "No boot log yet." : "Loading boot log…")}</p>)}
    </details>
  );
}

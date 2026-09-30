import { useState } from "react";
import { api, type DiagnosticReport } from "../lib/tauri";

type ReportState =
  | { readonly kind: "idle" }
  | { readonly kind: "saving" }
  | { readonly kind: "saved"; readonly report: DiagnosticReport; readonly copied: boolean }
  | { readonly kind: "failed"; readonly message: string };

export interface DiagnosticReportButtonProps {
  /** Tests and the shell lab only. */
  readonly save?: () => Promise<DiagnosticReport>;
  readonly className?: string;
}

/**
 * "Save a report for help": one file of technical facts (see
 * src-tauri/src/diagnostics.rs) in the Downloads folder, for whoever helps
 * the person. It never holds screenshots, tasks, keys or personal files,
 * and says so before anyone presses it.
 */
export function DiagnosticReportButton({ save = api.saveDiagnosticReport, className }: DiagnosticReportButtonProps) {
  const [state, setState] = useState<ReportState>({ kind: "idle" });
  const run = async () => {
    setState({ kind: "saving" });
    try {
      setState({ kind: "saved", report: await save(), copied: false });
    } catch (error) {
      setState({ kind: "failed", message: error instanceof Error ? error.message : String(error) });
    }
  };
  const copy = async (report: DiagnosticReport) => {
    try {
      await navigator.clipboard.writeText(report.text);
      setState({ kind: "saved", report, copied: true });
    } catch {
      setState({ kind: "saved", report, copied: false });
    }
  };
  return (
    <div className={className ? `report ${className}` : "report"}>
      <div className="report__actions">
        <button type="button" className="btn btn--small btn--line" disabled={state.kind === "saving"} onClick={() => void run()}>
          {state.kind === "saving" ? "Saving…" : state.kind === "saved" ? "Save again" : "Save a report for help"}
        </button>
        {state.kind === "saved" && (
          <button type="button" className="btn btn--small btn--quiet" onClick={() => void copy(state.report)}>
            {state.copied ? "Copied" : "Copy report"}
          </button>
        )}
      </div>
      <p className="report__note" role="status">
        {state.kind === "saved"
          ? <>Saved as <span className="mono">{state.report.file_name}</span> in <span className="mono">{state.report.location}</span>. Send that file to whoever is helping you.</>
          : state.kind === "failed"
            ? <>Pegoles couldn’t save the report: {state.message}</>
            : "Technical facts only: no screenshots, tasks, keys or personal files."}
      </p>
    </div>
  );
}

import { useState } from "react";
import { ChevronDownIcon, GlobeIcon } from "../ui/icons";
import type { InternetStatus } from "../lib/tauri";
import { decisionLine, latest, scopeText } from "./model";

const SHOWN = 6;

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * While a task is online this stays on screen: the mode, the sites, and a
 * live list of what Pegoles' proxy decided (host, verdict, reason, size).
 * Hosts come from the computer, so they are text only.
 */
export function InternetIndicator({ internet }: { readonly internet: InternetStatus }) {
  const [open, setOpen] = useState(true);
  if (!internet.active) return null;
  const rows = latest(internet.recent, SHOWN);
  const scope = scopeText(internet.mode, internet.domains);
  return (
    <section className="netbar" data-mode={internet.mode} aria-label="Internet access">
      <button type="button" className="netbar__head" aria-expanded={open} aria-controls="netbar-list" onClick={() => setOpen((value) => !value)}>
        <span className="netbar__icon" aria-hidden="true"><GlobeIcon size={14} /></span>
        <span className="netbar__text">
          <span className="netbar__title">Internet is on for this task</span>
          <span className="netbar__scope" title={scope}>{scope}</span>
        </span>
        <span className="netbar__count mono">{internet.allowed} allowed · {internet.blocked} blocked</span>
        <ChevronDownIcon size={14} className="netbar__chevron" />
      </button>
      {open && (
        <div id="netbar-list" className="netbar__body">
          {rows.length === 0 ? (
            <p className="netbar__empty">Nothing requested yet.</p>
          ) : (
            <ul className="netbar__list" aria-label="Recent requests, newest first">
              {rows.map((decision, index) => {
                const line = decisionLine(decision);
                return (
                  <li key={`${decision.at}-${index}`} className="netbar__row" data-verdict={line.verdict}>
                    <span className="netbar__verdict">{line.label}</span>
                    <span className="netbar__host mono" title={line.host}>{line.host}</span>
                    <span className="netbar__reason">{line.verdict === "allowed" ? size(decision.bytes) : line.text}</span>
                  </li>
                );
              })}
            </ul>
          )}
          {internet.dropped > 0 && <p className="netbar__empty">{internet.dropped} older requests were not listed.</p>}
        </div>
      )}
    </section>
  );
}

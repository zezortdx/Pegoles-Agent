import { useId, useState, type ClipboardEvent, type KeyboardEvent } from "react";
import { SegmentedControl, type Segment } from "../ui/SegmentedControl";
import { CloseIcon, GlobeIcon } from "../ui/icons";
import type { InternetMode } from "../lib/tauri";
import { MAX_SITES, chipLabel, parseSite, splitSites, type InternetDraft } from "./model";

const SEGMENTS: readonly Segment<InternetMode>[] = [
  { value: "off", label: "Off" },
  { value: "allowlist", label: "Only these sites" },
  { value: "open_web", label: "Open web" },
];

export const OPEN_WEB_WARNING =
  "Open web lets the agent reach almost any public site. Known malware, phishing, adult and gambling sites and downloads of programs or archives stay blocked, but a page can still be unsafe, and whatever the agent types into a page can leave its computer. Don’t give it passwords or secrets.";

export interface InternetChipProps {
  readonly draft: InternetDraft;
  readonly open: boolean;
  readonly onToggle: () => void;
}

/** The composer chip: what internet this task gets. Opens the panel. */
export function InternetChip({ draft, open, onToggle }: InternetChipProps) {
  return (
    <button type="button" className="composer-chip" data-tone={draft.mode === "open_web" ? "attention" : undefined}
      aria-expanded={open} aria-controls="internet-panel" onClick={onToggle}
      title="Choose whether this task's computer can reach the internet. It is off unless you allow it."
      aria-label={`Internet for this task: ${chipLabel(draft)}. ${open ? "Close" : "Choose"}`}>
      <GlobeIcon size={14} />
      <span>{chipLabel(draft)}</span>
    </button>
  );
}

export interface InternetPanelProps {
  readonly draft: InternetDraft;
  readonly onChange: (draft: InternetDraft) => void;
  readonly onClose: () => void;
}

/** The choice itself: Off (default) / Only these sites / Open web. */
export function InternetPanel({ draft, onChange, onClose }: InternetPanelProps) {
  const [text, setText] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const inputId = useId();

  const add = (raw: string) => {
    const parts = splitSites(raw);
    if (parts.length === 0) { setText(""); return; }
    const sites = [...draft.sites];
    let issue: string | null = null;
    for (const part of parts) {
      const parsed = parseSite(part);
      if ("problem" in parsed) { issue = parsed.problem; continue; }
      if (sites.includes(parsed.site)) continue;
      if (sites.length >= MAX_SITES) { issue = `Up to ${MAX_SITES} sites.`; break; }
      sites.push(parsed.site);
    }
    setProblem(issue);
    setText(issue && sites.length === draft.sites.length ? raw : "");
    if (sites.length !== draft.sites.length) onChange({ ...draft, sites });
  };
  const remove = (site: string) => onChange({ ...draft, sites: draft.sites.filter((s) => s !== site) });
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" || event.key === "," || event.key === " ") {
      event.preventDefault();
      add(text);
    } else if (event.key === "Backspace" && text === "" && draft.sites.length > 0) {
      remove(draft.sites[draft.sites.length - 1]);
    } else if (event.key === "Escape") {
      event.stopPropagation();
      onClose();
    }
  };
  const paste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData("text");
    if (splitSites(pasted).length > 1) { event.preventDefault(); add(pasted); }
  };

  return (
    <section id="internet-panel" className="internet-panel" aria-label="Internet for this task">
      <div className="internet-panel__head">
        <span className="internet-panel__title">Internet for this task</span>
        <button type="button" className="icon-btn" aria-label="Close internet options" onClick={onClose}><CloseIcon size={14} /></button>
      </div>
      <SegmentedControl id="internet-mode" size="small" label="Internet access" segments={SEGMENTS} value={draft.mode}
        onChange={(mode) => { setProblem(null); onChange({ ...draft, mode }); }} />
      {draft.mode === "off" && (
        <p className="internet-panel__note">The computer stays offline. You can allow sites for one task at a time.</p>
      )}
      {draft.mode === "allowlist" && (
        <div className="internet-panel__sites">
          <ul className="site-chips" aria-label="Allowed sites">
            {draft.sites.map((site) => (
              <li key={site} className="site-chip">
                <span className="site-chip__name mono">{site}</span>
                <button type="button" className="site-chip__remove" aria-label={`Remove ${site}`} onClick={() => remove(site)}>
                  <CloseIcon size={12} />
                </button>
              </li>
            ))}
            <li className="site-chips__entry">
              <label className="visually-hidden" htmlFor={inputId}>Add a site</label>
              <input id={inputId} className="site-input" value={text} placeholder={draft.sites.length === 0 ? "example.com" : "Add another site"}
                autoComplete="off" autoCapitalize="off" spellCheck={false} inputMode="url"
                aria-invalid={problem ? true : undefined} aria-describedby={problem ? `${inputId}-problem` : undefined}
                disabled={draft.sites.length >= MAX_SITES}
                onChange={(event) => { setText(event.target.value); if (problem) setProblem(null); }}
                onKeyDown={keys} onPaste={paste} onBlur={() => { if (text.trim()) add(text); }} />
            </li>
          </ul>
          <div className="internet-panel__meta">
            <span>Each site also covers its subdomains. Everything else is blocked.</span>
            <span className="mono">{draft.sites.length} / {MAX_SITES}</span>
          </div>
          {problem && <p id={`${inputId}-problem`} className="internet-panel__problem" role="alert">{problem}</p>}
        </div>
      )}
      {draft.mode === "open_web" && <p className="internet-panel__warning" role="note">{OPEN_WEB_WARNING}</p>}
      {draft.mode !== "off" && <p className="internet-panel__note">Pegoles asks you to confirm in its own window when the task starts. It ends when the task stops or you take control.</p>}
    </section>
  );
}

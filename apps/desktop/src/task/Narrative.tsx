import { useEffect, useId, useState } from "react";
import { AnimatePresence, m } from "motion/react";
import type { TranscriptItem, TranscriptStep } from "../state/transcript";
import { FOLD_FROM, noteIsLong, pinnedSteps, runSummary } from "../state/narrative";
import type { AgentMessageKind } from "../lib/tauri";
import { CapabilityGlyph } from "../artifacts/glyphs";
import { basename, formatDuration, parentOf, plural, timeOf } from "../artifacts/format";
import { duration, ease } from "../lib/motion";
import { AlertIcon, CheckIcon, ChevronRightIcon, CloseIcon, FileIcon } from "../ui/icons";

type Item = Exclude<TranscriptItem, { kind: "request" }>;

const OUTCOME_WORD: Partial<Record<TranscriptStep["outcome"], string>> = {
  blocked: "Blocked by a safety rule",
  failed: "Didn’t work",
  interrupted: "Interrupted",
  approval: "Needs approval",
};

/** A workspace path reads as its file name; the full path stays in the tooltip. */
function shortDetail(detail: string): string {
  if (!detail.startsWith("/") || detail.includes(" ")) return detail;
  return basename(detail);
}

export function StepRow({ step }: { step: TranscriptStep }) {
  const word = OUTCOME_WORD[step.outcome];
  return (
    <div className="step" data-outcome={step.outcome} data-consequential={step.consequential || undefined}>
      <span className="step__icon" aria-hidden="true"><CapabilityGlyph capability={step.capability} size={14} /></span>
      <span className="step__text" title={step.detail ?? step.label}>
        <span className="step__label">{step.label}</span>
        {step.detail && <span className="step__detail">{shortDetail(step.detail)}</span>}
        {word && <span className="step__word" data-outcome={step.outcome}>{word}</span>}
      </span>
      {typeof step.durationMs === "number" && <span className="step__meta">{formatDuration(step.durationMs)}</span>}
    </div>
  );
}

/** A run of finished actions: short runs read open, long ones fold into one line that still shows what changed. */
function Run({ steps, at, current }: { steps: readonly TranscriptStep[]; at: string; current: boolean }) {
  // The run Pegoles is in the middle of reads open; finished runs fold to one line.
  const [open, setOpen] = useState(current);
  useEffect(() => { if (!current) setOpen(false); }, [current]);
  const listId = useId();
  if (steps.length < FOLD_FROM) {
    return <div className="run run--open">{steps.map((step) => <StepRow key={step.id} step={step} />)}</div>;
  }
  const summary = runSummary(steps);
  const pinned = pinnedSteps(steps);
  const meta = [plural(summary.count, "action"), summary.durationMs !== undefined ? formatDuration(summary.durationMs) : null].filter(Boolean).join(" · ");
  const shown = open ? steps : pinned;
  return (
    <div className="run" data-open={open || undefined}>
      <button type="button" className="run__summary" aria-expanded={open} aria-controls={listId} onClick={() => setOpen((value) => !value)}>
        <ChevronRightIcon size={12} className="run__chevron" />
        <span className="run__label">{summary.label}</span>
        <span className="run__meta">{meta}</span>
        <time className="run__time" dateTime={at}>{timeOf(at)}</time>
      </button>
      <div id={listId} className="run__steps">
        <AnimatePresence initial={false}>
          {shown.map((step) => (
            <m.div key={step.id}
              initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }}
              transition={{ duration: duration.surface, ease: ease.out }} style={{ overflow: "hidden" }}>
              <StepRow step={step} />
            </m.div>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}

function FileItem({ path, at }: { path: string; at: string }) {
  const name = basename(path);
  return (
    <article className="made" aria-label={`File created: ${name}`} title={path}>
      <span className="made__tile" aria-hidden="true"><FileIcon size={16} /></span>
      <span className="made__body">
        <span className="made__name">{name}</span>
        <span className="made__path">{parentOf(path)}</span>
      </span>
      <span className="made__meta">Created <time dateTime={at}>{timeOf(at)}</time></span>
    </article>
  );
}

export const APPROVAL_NOTE = "Nothing has been allowed. This build can’t answer approvals yet, so the task stays paused here.";

function Approval({ reason, open, at }: { reason?: string; open: boolean; at: string }) {
  const titleId = useId();
  if (!open) {
    return (
      <div className="step" data-outcome="approval">
        <span className="step__icon" aria-hidden="true"><AlertIcon size={14} /></span>
        <span className="step__text"><span className="step__label">Asked for approval{reason ? `: ${reason}` : ""}</span></span>
        <time className="step__meta" dateTime={at}>{timeOf(at)}</time>
      </div>
    );
  }
  return (
    <article className="ask" aria-labelledby={titleId}>
      <header className="ask__head">
        <span className="ask__dot" aria-hidden="true" />
        <h3 id={titleId} className="ask__title">Stopped to ask you</h3>
        <time className="ask__time" dateTime={at}>{timeOf(at)}</time>
      </header>
      <p className="ask__reason">{reason ?? "An action needs your approval before Pegoles continues."}</p>
      <p className="ask__note">{APPROVAL_NOTE}</p>
    </article>
  );
}

const NOTE_LABEL: Record<AgentMessageKind, string> = {
  progress: "Note from Pegoles",
  summary: "Pegoles’ summary",
  error: "Why Pegoles stopped",
};

/**
 * Pegoles' own words. Model text is untrusted: it is only ever a text node
 * (never markup), and long notes fold to a few lines until asked.
 */
function Note({ item }: { item: Extract<Item, { kind: "message" }> }) {
  const [open, setOpen] = useState(false);
  const textId = useId();
  const long = noteIsLong(item.text);
  return (
    <article className="note" data-variant={item.variant} aria-label={NOTE_LABEL[item.variant]}>
      {item.variant === "error" && <span className="note__icon" aria-hidden="true"><AlertIcon size={14} /></span>}
      <div className="note__body">
        <p id={textId} className="note__text" data-folded={(long && !open) || undefined}>{item.text}</p>
        {long && (
          <button type="button" className="note__more" aria-expanded={open} aria-controls={textId} onClick={() => setOpen((value) => !value)}>
            {open ? "Show less" : "Show more"}
          </button>
        )}
      </div>
    </article>
  );
}

const OUTCOME_TITLE = { completed: "Done", failed: "Couldn’t finish", cancelled: "Cancelled" } as const;

function Outcome({ item, detail }: { item: Extract<Item, { kind: "outcome" }>; detail?: string }) {
  const facts = [plural(item.actions, "action"), item.files ? plural(item.files, "file") : null, `finished ${timeOf(item.at)}`].filter(Boolean).join(" · ");
  const Glyph = item.status === "completed" ? CheckIcon : item.status === "failed" ? CloseIcon : AlertIcon;
  return (
    <section className="outcome" data-status={item.status} aria-label="Finished">
      <span className="outcome__tile" aria-hidden="true"><Glyph size={16} /></span>
      <div className="outcome__text">
        <h3 className="outcome__title">{OUTCOME_TITLE[item.status]}</h3>
        <p className="outcome__facts">{facts}</p>
        {item.status === "failed" && detail && <p className="outcome__detail">Stopped at: {detail}</p>}
      </div>
    </section>
  );
}

function ItemView({ item, failedAt, current }: { item: Item; failedAt?: string; current: boolean }) {
  switch (item.kind) {
    case "actions": return <Run steps={item.steps} at={item.at} current={current} />;
    case "file": return <FileItem path={item.path} at={item.at} />;
    case "approval": return <Approval reason={item.reason} open={item.open} at={item.at} />;
    case "message": return <Note item={item} />;
    case "outcome": return <Outcome item={item} detail={failedAt} />;
  }
}

export interface NarrativeProps {
  readonly items: readonly TranscriptItem[];
  readonly animated: boolean;
  /** Pegoles is still working: its latest run stays open. */
  readonly live: boolean;
  /** For a failed task: the step it stopped at. */
  readonly failedAt?: string;
}

/**
 * The work as a readable story, oldest first: what Pegoles wrote, runs of
 * actions (folded when long, never hiding what changed), files it made,
 * where it stopped to ask, and how it ended. New items rise in; nothing
 * reflows under the reader.
 */
export function Narrative({ items, animated, failedAt, live }: NarrativeProps) {
  const body = items.filter((item): item is Item => item.kind !== "request");
  if (!body.length) return null;
  const last = body[body.length - 1];
  return (
    <ol className="narrative" aria-label="Task timeline">
      <AnimatePresence initial={false}>
        {body.map((item) => (
          <m.li key={item.id} className="narrative__item" data-kind={item.kind}
            initial={animated ? { opacity: 0, y: 8 } : { opacity: 0 }}
            animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}>
            <ItemView item={item} failedAt={failedAt} current={live && item === last} />
          </m.li>
        ))}
      </AnimatePresence>
    </ol>
  );
}

import { useId, useMemo, useState, type Ref } from "react";
import {
  activityStream, activitySummary, clockTime, filterDays, GROUP_STATUS, tasksWithoutActivity,
  type ActivityEntry, type ActivityFilter, type ActivityGroup, type ActivitySummary, type EntryOutcome, type GroupStatusTone,
} from "../state/activityStream";
import type { AgentEvent, AgentTask } from "../lib/tauri";
import { AlertIcon, CheckIcon, CloseIcon } from "../ui/icons";
import { SegmentedControl } from "../ui/SegmentedControl";

export interface ActivityViewProps {
  readonly events: readonly AgentEvent[];
  readonly tasks: readonly AgentTask[];
  readonly headingRef?: Ref<HTMLHeadingElement>;
  /** Optional: makes tasks listed in the empty state open their task. */
  readonly onOpenTask?: (task: AgentTask) => void;
}

const FILTERS = [
  { value: "all", label: "All" },
  { value: "needs-you", label: "Needs you" },
  { value: "computer", label: "Computer" },
] as const satisfies readonly { value: ActivityFilter; label: string }[];

const FILTER_EMPTY: Record<Exclude<ActivityFilter, "all">, string> = {
  "needs-you": "Nothing needs you right now. Tasks waiting for your approval show up here.",
  computer: "Its computer hasn’t reported anything yet. Starting, stopping and connection changes show up here.",
};

/** Runs longer than this fold their earlier steps. */
const FOLD_AFTER = 8;
const FOLD_SHOW = 6;

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

function summaryLine(summary: ActivitySummary, taskCount: number): string {
  if (summary.scope === "none") return taskCount ? `${plural(taskCount, "task")} · nothing recorded yet` : "Nothing recorded yet";
  const parts = [summary.tasks ? plural(summary.tasks, "task") : null, summary.actions ? plural(summary.actions, "action") : null]
    .filter(Boolean).join(" · ");
  const counted = parts ? `${parts}${summary.scope === "today" ? " today" : ""}` : summary.scope === "today" ? "Computer activity today" : "Computer activity";
  return summary.needsYou ? `${counted} · ${summary.needsYou} needs you` : counted;
}

const OUTCOME_WORD: Record<EntryOutcome, string | null> = {
  done: "done", running: "in progress", attention: "needs attention", error: "didn’t work", neutral: null,
};

function RowGlyph({ outcome }: { outcome: EntryOutcome }) {
  if (outcome === "done") return <CheckIcon size={12} className="arow__glyph" data-outcome="done" />;
  if (outcome === "error") return <CloseIcon size={12} className="arow__glyph" data-outcome="error" />;
  if (outcome === "attention") return <AlertIcon size={12} className="arow__glyph" data-outcome="attention" />;
  return <span className="arow__glyph arow__glyph--dot pg-work-anim" data-outcome={outcome} aria-hidden="true" />;
}

function ActivityRow({ entry }: { entry: ActivityEntry }) {
  const [open, setOpen] = useState(false);
  const detailsId = useId();
  const word = OUTCOME_WORD[entry.outcome];
  return (
    <li className="arow" data-outcome={entry.outcome} data-open={open || undefined}>
      <RowGlyph outcome={entry.outcome} />
      <span className="arow__text" title={entry.context ? `${entry.text} · ${entry.context}` : entry.text}>
        {entry.text}
        {entry.context && <span className="arow__context">{entry.context}</span>}
        {word && <span className="visually-hidden">, {word}</span>}
      </span>
      <button type="button" className="arow__more" aria-expanded={open} aria-controls={detailsId} onClick={() => setOpen((value) => !value)}>
        Details
      </button>
      <time className="arow__time" dateTime={entry.at}>{clockTime(entry.at)}</time>
      {open && <pre id={detailsId} className="diagnostic arow__details">{entry.details}</pre>}
    </li>
  );
}

const STATUS_DOT: Record<GroupStatusTone, string | undefined> = {
  active: "live", attention: "attention", done: "done", error: "error", quiet: undefined,
};

function GroupStatus({ group }: { group: ActivityGroup }) {
  if (!group.statusLabel || !group.statusTone) return null;
  const live = group.status === "running";
  return (
    <span className="agroup__status" data-tone={group.statusTone}>
      <span className="state-dot" data-tone={STATUS_DOT[group.statusTone]} data-live={live || undefined} aria-hidden="true" />
      {group.statusLabel}
    </span>
  );
}

function Group({ group, onOpen }: { group: ActivityGroup; onOpen?: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const titleId = useId();
  const folded = group.entries.length > FOLD_AFTER && !expanded;
  const shown = folded ? group.entries.slice(-FOLD_SHOW) : group.entries;
  return (
    <li className="agroup" data-kind={group.kind} aria-labelledby={titleId}>
      <div className="agroup__head">
        <span className="agroup__node" data-kind={group.kind} data-tone={group.statusTone ?? undefined} aria-hidden="true" />
        {onOpen ? (
          <button type="button" id={titleId} className="agroup__title agroup__title--link" title={`Open “${group.title}”`} onClick={onOpen}>{group.title}</button>
        ) : (
          <h3 id={titleId} className="agroup__title" title={group.title}>{group.title}</h3>
        )}
        <GroupStatus group={group} />
        <time className="agroup__time" dateTime={group.at}>{clockTime(group.at)}</time>
      </div>
      {folded && (
        <button type="button" className="agroup__more" onClick={() => setExpanded(true)}>
          Show {plural(group.entries.length - shown.length, "earlier step")}
        </button>
      )}
      <ol className="agroup__rows">{shown.map((entry) => <ActivityRow key={entry.id} entry={entry} />)}</ol>
    </li>
  );
}

function EmptyActivity({ tasks, onOpenTask }: { tasks: readonly AgentTask[]; onOpenTask?: (task: AgentTask) => void }) {
  return (
    <div className="activity-empty">
      <p className="activity-empty__title">No activity yet</p>
      <p className="activity-empty__text">
        When Pegoles works, every step lands here, grouped by task: files it reads and writes, websites it visits,
        approvals it asks for, and when its computer starts or stops.
      </p>
      {tasks.length > 0 && (
        <section className="activity-quiet" aria-labelledby="activity-quiet-title">
          <h2 id="activity-quiet-title" className="page__section-title">Tasks with no activity yet</h2>
          <ul className="activity-quiet__list">
            {tasks.map((task) => {
              const body = (
                <>
                  <span className="activity-quiet__title">{task.title}</span>
                  <span className="activity-quiet__status">{GROUP_STATUS[task.status].label}</span>
                </>
              );
              return (
                <li key={task.id}>
                  {onOpenTask
                    ? <button type="button" className="activity-quiet__row" onClick={() => onOpenTask(task)}>{body}</button>
                    : <div className="activity-quiet__row">{body}</div>}
                </li>
              );
            })}
          </ul>
        </section>
      )}
    </div>
  );
}

/** What happened, grouped by day and by task run. Technical data folds away. */
export function ActivityView({ events, tasks, headingRef, onOpenTask }: ActivityViewProps) {
  const [filter, setFilter] = useState<ActivityFilter>("all");
  const days = useMemo(() => activityStream(events, tasks), [events, tasks]);
  const summary = useMemo(() => activitySummary(days, tasks), [days, tasks]);
  const shown = useMemo(() => filterDays(days, filter), [days, filter]);
  const quiet = useMemo(() => (days.length ? [] : tasksWithoutActivity(days, tasks)), [days, tasks]);
  const byId = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  return (
    <section className="page page--activity" aria-labelledby="activity-title">
      <header className="page__head">
        <div className="page__heading">
          <h1 id="activity-title" ref={headingRef} tabIndex={-1} className="page__title">Activity</h1>
          <p className="page__summary">{summaryLine(summary, tasks.length)}</p>
        </div>
        {days.length > 0 && (
          <SegmentedControl id="activity-filter" label="Show activity" segments={FILTERS} value={filter} onChange={setFilter} size="small" />
        )}
      </header>
      {days.length === 0 ? <EmptyActivity tasks={quiet} onOpenTask={onOpenTask} /> : shown.length === 0 ? (
        <p className="page__empty">{FILTER_EMPTY[filter as Exclude<ActivityFilter, "all">]}</p>
      ) : shown.map((day) => (
        <section key={day.key} className="activity-day" aria-label={day.label}>
          <h2 className="page__section-title">{day.label}</h2>
          <ol className="activity-day__groups">
            {day.groups.map((group) => {
              const task = group.taskId ? byId.get(group.taskId) : undefined;
              return <Group key={group.key} group={group} onOpen={task && onOpenTask ? () => onOpenTask(task) : undefined} />;
            })}
          </ol>
        </section>
      ))}
    </section>
  );
}

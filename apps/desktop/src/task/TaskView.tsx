import { useEffect, useRef, type Ref } from "react";
import { m } from "motion/react";
import type { AgentTask } from "../lib/tauri";
import type { TaskActivity } from "../state/agentState";
import type { HumanError } from "../state/errors";
import type { TranscriptItem } from "../state/transcript";
import type { ActivityPill } from "../lib/taskState";
import { canSetUp, progressText, sentence, setupAction, type LocalView } from "../state/localModel";
import { duration, ease } from "../lib/motion";
import { plural, timeOf } from "../artifacts/format";
import { LocalProgress } from "../intelligence/LocalProgress";
import { Narrative } from "./Narrative";
import { HOST } from "../lib/host";

export interface TaskViewProps {
  readonly task: AgentTask;
  readonly items: readonly TranscriptItem[];
  readonly activity: TaskActivity;
  readonly state: ActivityPill;
  readonly elapsed?: string;
  readonly animated: boolean;
  /** Just handed over from Home. */
  readonly arriving: boolean;
  readonly headingRef?: Ref<HTMLHeadingElement>;
  /** The objective is on screen (the toolbar shows the title only once it isn't). */
  readonly onHeadingVisible: (visible: boolean) => void;
  readonly onOpenComputer: () => void;
  readonly onModelSettings: () => void;
  /** Start the agent on this task (only offered while it hasn't started). */
  readonly onStart: () => void;
  /** A start is in flight. */
  readonly starting?: boolean;
  /** Why the last start of this task didn't happen. */
  readonly startError?: HumanError | null;
  /** Pegoles Local, while the task waits for it: its state and a way to set it up right here. */
  readonly local?: LocalSetup;
}

/** Pegoles Local as a task waiting for it sees it. */
export interface LocalSetup {
  readonly view: LocalView;
  /** A set-up or cancel is on its way to Core. */
  readonly busy: boolean;
  /** Why the last set-up or cancel didn't happen, in Core's words. */
  readonly problem?: string | null;
  readonly onSetUp: () => void;
  readonly onCancel: () => void;
}

function actionCount(items: readonly TranscriptItem[]): number {
  return items.reduce((sum, item) => sum + (item.kind === "actions" ? item.steps.length : item.kind === "file" ? 1 : 0), 0);
}

type NotStartedProps = Pick<TaskViewProps, "activity" | "onOpenComputer" | "onModelSettings" | "onStart" | "starting" | "startError" | "local">;

const NOT_STARTED_TITLE = { "needs-model": "Waiting for a model", ready: "Not started yet", busy: "Pegoles is busy" } as const;
const NOT_STARTED_BODY = {
  ready: "Pegoles will do this on its own computer once you start it.",
  busy: "Pegoles is working on another task. It works on one at a time, so start this one when that one ends.",
} as const;
const PREPARING_BODY = `Pegoles is getting its local model ready on this ${HOST}. Start this task once it’s ready; it’s kept while the app is open.`;

/** The set-up button's words on a task: the product's name first, since the task doesn't say where it is. */
function setupLabel(view: LocalView): string {
  if (view.stage === "not-set-up") return "Set up Pegoles Local";
  if (view.stage === "paused") return "Resume setup";
  return setupAction(view);
}

/** What stands between this task and Pegoles Local, in one line under the notice. */
function LocalLine({ setup }: { setup: LocalSetup }) {
  const { view } = setup;
  switch (view.stage) {
    case "preparing": return <div className="notice__progress"><LocalProgress view={view} /></div>;
    case "failed": return <p className="notice__problem" role="alert">{sentence(view.error ?? "The setup didn’t finish.")}</p>;
    case "paused": return <p className="notice__meta">Paused at {progressText(view.doneBytes ?? 0, view.totalBytes ?? 0)}. It picks up where it stopped.</p>;
    case "damaged": return <p className="notice__meta">Its files didn’t pass the check. Setting it up again replaces them.</p>;
    case "unsupported":
    case "downloaded": return <p className="notice__meta">{view.problem ?? "Pegoles Local can’t run on this computer."}</p>;
    default: return view.problem ? <p className="notice__meta">{view.problem}</p> : null;
  }
}

/** The task hasn't started: said once, where the work would be, with the real ways forward. */
function NotStarted({ activity, onOpenComputer, onModelSettings, onStart, starting = false, startError, local }: NotStartedProps) {
  const start = activity.start ?? "ready";
  const needsModel = start === "needs-model";
  const setup = needsModel && activity.waitingFor !== "cloud-key" ? local : undefined;
  const preparing = setup?.view.stage === "preparing";
  const body = start === "needs-model" ? (preparing ? PREPARING_BODY : activity.detail) : NOT_STARTED_BODY[start];
  return (
    <div className="notice" data-tone="quiet">
      <p className="notice__title">{preparing ? "Setting up Pegoles Local" : NOT_STARTED_TITLE[start]}</p>
      <p className="notice__body">{body}</p>
      {setup && <LocalLine setup={setup} />}
      {setup?.problem && <p className="notice__problem" role="alert">{setup.problem}</p>}
      {startError && (
        <p className="notice__problem" role="alert">{startError.hint ? `${startError.title} ${startError.hint}` : startError.title}</p>
      )}
      <div className="notice__actions">
        {needsModel ? (
          <>
            {setup && canSetUp(setup.view) && (
              <button type="button" className="btn btn--primary btn--small" disabled={setup.busy} onClick={setup.onSetUp}>{setupLabel(setup.view)}</button>
            )}
            {setup && preparing && (
              <button type="button" className="btn btn--quiet btn--small" disabled={setup.busy} onClick={setup.onCancel}>Cancel setup</button>
            )}
            <button type="button" className="btn btn--line btn--small" onClick={onModelSettings}>Open Settings</button>
          </>
        ) : (
          <button type="button" className="btn btn--primary btn--small" disabled={start === "busy" || starting} onClick={onStart}>
            {starting ? <><span className="spinner" aria-hidden="true" />Starting…</> : "Start"}
          </button>
        )}
        <button type="button" className="btn btn--quiet btn--small" onClick={onOpenComputer}>Open its computer</button>
      </div>
    </div>
  );
}

/**
 * A task is a work session, not a conversation: the objective in your
 * words, the facts of its state, then the work as it happened. What
 * Pegoles is doing right now lives in the status bar below.
 */
export function TaskView(props: TaskViewProps) {
  const { task, items, activity, state } = props;
  const headRef = useRef<HTMLElement>(null);
  const { onHeadingVisible } = props;
  useEffect(() => {
    const head = headRef.current;
    if (!head || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => onHeadingVisible(entry.isIntersecting), { threshold: 0, rootMargin: "-52px 0px 0px 0px" });
    observer.observe(head);
    return () => { observer.disconnect(); onHeadingVisible(true); };
  }, [onHeadingVisible]);

  const actions = actionCount(items);
  const notStarted = task.status === "pending" && !!activity.start;
  const waitingFirst = activity.live && !notStarted && activity.mode !== "needs-user" && items.length <= 1;
  const facts = [
    `${task.status === "pending" ? "Created" : "Started"} ${timeOf(task.created_at)}`,
    props.elapsed && activity.live ? props.elapsed : null,
    actions ? plural(actions, "action") : null,
  ].filter(Boolean) as string[];
  const enter = props.arriving && props.animated
    ? { initial: { opacity: 0, y: 10 }, animate: { opacity: 1, y: 0, transition: { duration: duration.spatial, ease: ease.out, delay: 0.06 } } }
    : {};

  return (
    <article className="task" aria-labelledby="task-title">
      <div className="task__scroll">
        <div className="task__column">
          <m.header ref={headRef} className="task-head" {...enter}>
            <h1 id="task-title" ref={props.headingRef} tabIndex={-1} className="task-head__title">{task.title}</h1>
            <p className="task-head__meta">
              <span className="state-label" data-tone={state.tone}>
                <span className="state-dot" data-tone={state.tone} data-live={state.live || undefined} aria-hidden="true" />
                {state.label}
              </span>
              {facts.map((fact) => <span key={fact} className="task-head__fact">{fact}</span>)}
            </p>
          </m.header>

          {notStarted && (
            <NotStarted activity={activity} onOpenComputer={props.onOpenComputer} onModelSettings={props.onModelSettings}
              onStart={props.onStart} starting={props.starting} startError={props.startError} local={props.local} />
          )}
          <Narrative items={items} animated={props.animated} live={activity.live} failedAt={task.status === "failed" ? activity.detail : undefined} />
          {waitingFirst && <p className="narrative__waiting"><span className="narrative__pulse pg-work-anim" aria-hidden="true" />Waiting for the first action</p>}
        </div>
      </div>
    </article>
  );
}

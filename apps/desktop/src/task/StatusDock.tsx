import { AnimatePresence, m } from "motion/react";
import { PresenceMark } from "../presence";
import type { TaskActivity } from "../state/agentState";
import type { ActivityPill } from "../lib/taskState";
import { duration, ease } from "../lib/motion";
import { basename, timeOf } from "../artifacts/format";
import { ComposeIcon, PanelRightIcon, StopIcon } from "../ui/icons";
import { PRESENCE_LAYOUT_ID } from "../home/HomeView";
import { useHeldValue } from "./useHeldValue";

export interface StatusDockProps {
  readonly activity: TaskActivity;
  readonly state: ActivityPill;
  readonly elapsed?: string;
  /** Pegoles just arrived from Home: its mark travels here. */
  readonly arriving: boolean;
  /** Core reports input in flight on its computer: it can be interrupted. */
  readonly interruptible: boolean;
  readonly interrupting: boolean;
  readonly computerOpen: boolean;
  readonly modifier: string;
  readonly onInterrupt: () => void;
  readonly onWatch: () => void;
  readonly onNewTask: () => void;
}

/** Every line stays readable: fast real changes are coalesced, never invented. */
const HOLD_MS = 600;
/** Modes where a light passes through the words, and only while live. */
const SHINE = new Set<TaskActivity["mode"]>(["thinking", "planning", "working", "using-computer", "acknowledging"]);

function detailOf(activity: TaskActivity): string | undefined {
  if (activity.mode === "needs-user") return activity.approvalReason;
  if (!activity.live && activity.at) return activity.mode === "error" && activity.detail ? `Stopped at: ${activity.detail}` : `Finished ${timeOf(activity.at)}`;
  if (activity.mode === "blocked" && activity.offerComputer) return "No model is connected";
  const detail = activity.detail;
  if (!detail) return undefined;
  return detail.startsWith("/") && !detail.includes(" ") ? basename(detail) : detail;
}

function headlineOf(activity: TaskActivity, state: ActivityPill): string {
  if (activity.mode === "blocked" && activity.offerComputer) return "Not started";
  if (!activity.live) return state.label;
  return activity.headline;
}

/**
 * Where the composer was, the job now reports: one line for what Pegoles
 * is doing and why, the time it has spent, and the one or two things you
 * can really do about it. The mark is the same Pegoles that took the job.
 */
export function StatusDock(props: StatusDockProps) {
  const { activity, state } = props;
  const headline = headlineOf(activity, state);
  const detail = detailOf(activity);
  const settledNow = !activity.live || (activity.mode === "blocked" && !!activity.offerComputer);
  const key = `${headline}·${detail ?? ""}·${settledNow}`;
  // The line and what it offers change together, so an action never outruns its words.
  const shown = useHeldValue({ headline, detail, settled: settledNow }, key, HOLD_MS);
  const shine = activity.live && SHINE.has(activity.mode);
  const settled = shown.settled;
  const watch = activity.mode === "using-computer" && !props.computerOpen;
  return (
    <div className="dock" role="region" aria-label="Now" data-mode={activity.mode} data-tone={state.tone}>
      <m.span className="dock__mark" layoutId={props.arriving ? PRESENCE_LAYOUT_ID : undefined}>
        <PresenceMark mode={activity.mode} size={22} decorative={false} />
      </m.span>
      <div className="dock__text" aria-live="polite">
        <AnimatePresence initial={false} mode="popLayout">
          <m.span
            key={shown.headline}
            className="dock__headline"
            data-shine={shine || undefined}
            data-text={shown.headline}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}
            exit={{ opacity: 0, y: -8, transition: { duration: duration.micro, ease: ease.exit } }}
          >
            {shown.headline}
          </m.span>
        </AnimatePresence>
        {shown.detail && <span className="dock__detail" title={shown.detail}>{shown.detail}</span>}
      </div>
      {props.elapsed && <span className="dock__elapsed" title="Time spent">{props.elapsed}</span>}
      <div className="dock__actions">
        {props.interruptible && (
          <button type="button" className="btn btn--quiet" disabled={props.interrupting} onClick={props.onInterrupt}
            title={`Stop what Pegoles is doing on its computer (${props.modifier}.)`}>
            <StopIcon size={12} />Stop
          </button>
        )}
        {watch && (
          <button type="button" className="btn btn--quiet" onClick={props.onWatch}>
            <PanelRightIcon size={14} />Watch
          </button>
        )}
        {settled && (
          <button type="button" className="btn btn--quiet" onClick={props.onNewTask} title={`New task (${props.modifier}N)`}>
            <ComposeIcon size={14} />New task
          </button>
        )}
      </div>
    </div>
  );
}

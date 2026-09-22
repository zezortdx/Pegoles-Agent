import type { ReactNode } from "react";
import { AnimatePresence, LayoutGroup, m } from "motion/react";
import { duration, easingBezier, spring } from "../tokens/motion.js";
import { radiusRole } from "../tokens/radius.js";
import { useFluxGlass } from "../runtime/context.js";
import { COMMAND_SURFACE_LAYOUT_ID } from "./CommandBar.js";
import { useGlassSurface } from "./GlassSurface.js";
import { ProgressLine } from "./ProgressLine.js";
import { StatusIndicator, type StatusTone } from "./StatusIndicator.js";
import { cx } from "./cx.js";
import "./command.css";

export interface TaskControllerProps {
  /** The task as the user asked it. */
  readonly title: string;
  /** Real task status from Core (never inferred by the UI). */
  readonly status: { readonly tone: StatusTone; readonly label: string };
  /** Current step or last meaningful event, e.g. "Opening a terminal". */
  readonly detail?: string;
  /** Preformatted real elapsed time, e.g. "0:42". */
  readonly elapsed?: string;
  /** Leading slot — typically the PegolesMark in its working state. */
  readonly leading?: ReactNode;
  /** Actions slot (Pause / Stop / Open), GlassButtons. */
  readonly actions?: ReactNode;
  /** Real work in flight: shows an indeterminate work line. */
  readonly working?: boolean;
  /** Real completion fraction 0..1, when one exists. Never invent one. */
  readonly progress?: number | null;
  /** Morph identity; share it with the CommandBar this replaces. */
  readonly layoutId?: string;
  readonly className?: string;
}

/**
 * What the CommandBar becomes once a task exists: same radius, same
 * position, electric material (Pegoles is present and working).
 */
export function TaskController({
  title,
  status,
  detail,
  elapsed,
  leading,
  actions,
  working = false,
  progress = null,
  layoutId = COMMAND_SURFACE_LAYOUT_ID,
  className,
}: TaskControllerProps) {
  const { transitionStyle } = useFluxGlass();
  const glass = useGlassSurface("electric", "TaskController");
  const morph = transitionStyle === "morph";
  const hasProgress = typeof progress === "number" && Number.isFinite(progress);

  return (
    <m.section
      ref={glass.ref}
      layoutId={morph ? layoutId : undefined}
      transition={{ layout: spring.morph }}
      style={{ borderRadius: radiusRole.command }}
      className={cx("pg-glass", "pg-task", className)}
      {...glass.attributes}
      data-elevation="3"
      data-radius="xxl"
      aria-label="Current task"
    >
      <m.div
        className="pg-task__body"
        layout="position"
        initial={morph ? { opacity: 0 } : false}
        animate={{ opacity: 1 }}
        transition={{ duration: duration.interaction / 1000, delay: morph ? 0.08 : 0, ease: easingBezier.out }}
      >
        {leading && (
          <span className="pg-task__leading" aria-hidden="true">
            {leading}
          </span>
        )}
        <div className="pg-task__text">
          <p className="pg-task__title">{title}</p>
          <div className="pg-task__meta">
            <StatusIndicator tone={status.tone} label={status.label} size="sm" live />
            {detail && <span className="pg-task__detail">{detail}</span>}
            {elapsed && <span className="pg-task__elapsed">{elapsed}</span>}
          </div>
        </div>
        {actions && <div className="pg-task__actions">{actions}</div>}
      </m.div>
      {(working || hasProgress) && (
        <ProgressLine className="pg-task__progress" value={hasProgress ? progress : null} label={`${title} progress`} />
      )}
    </m.section>
  );
}

export interface CommandMorphProps {
  /** `command` shows the CommandBar, `task` the TaskController. */
  readonly mode: "command" | "task";
  readonly command: ReactNode;
  readonly task: ReactNode;
  readonly className?: string;
}

const crossfade = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: { duration: duration.interaction / 1000, ease: easingBezier.out },
} as const;

/**
 * The CommandBar → TaskController transition.
 * - `morph` (Full/Reduced, no reduced motion): both surfaces share a
 *   layoutId, so the glass reshapes in place on a critically damped
 *   spring while the task content fades in.
 * - `crossfade` (reduced motion or Minimal): a 180 ms opacity crossfade;
 *   no movement, same information.
 */
export function CommandMorph({ mode, command, task, className }: CommandMorphProps) {
  const { transitionStyle } = useFluxGlass();
  if (transitionStyle === "morph") {
    return (
      <div className={cx("pg-command-morph", className)} data-transition="morph">
        <LayoutGroup id="pegoles-command-morph">{mode === "command" ? command : task}</LayoutGroup>
      </div>
    );
  }
  return (
    <div className={cx("pg-command-morph", className)} data-transition="crossfade">
      <AnimatePresence mode="popLayout" initial={false}>
        <m.div key={mode} className="pg-command-morph__slot" {...crossfade}>
          {mode === "command" ? command : task}
        </m.div>
      </AnimatePresence>
    </div>
  );
}

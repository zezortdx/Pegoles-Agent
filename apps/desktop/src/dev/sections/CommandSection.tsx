import { useEffect, useState } from "react";
import {
  CommandBar,
  CommandMorph,
  GlassButton,
  PauseIcon,
  PegolesMark,
  PlayIcon,
  StopIcon,
  TaskController,
  useFluxGlass,
  type StatusTone,
} from "@pegoles/ui";
import { TASK_STEPS } from "../fixtures";
import { LabSection, SimulatedTag } from "../lab/controls";

type Phase = "working" | "paused" | "done";

interface SimTask {
  readonly title: string;
  readonly step: number;
  readonly elapsedS: number;
  readonly phase: Phase;
}

const STEP_MS = 1800;

function formatElapsed(s: number): string {
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

const MARK_STATE = { working: "acting", paused: "idle", done: "success" } as const;

const STATUS: Readonly<Record<Phase, { tone: StatusTone; label: string }>> = {
  working: { tone: "active", label: "Working" },
  paused: { tone: "paused", label: "Paused" },
  done: { tone: "success", label: "Done" },
};

/** Lab-only simulation of a task advancing through steps. */
function useSimulatedTask() {
  const [task, setTask] = useState<SimTask | null>(null);
  const phase = task?.phase;

  useEffect(() => {
    if (phase !== "working") return undefined;
    const tick = window.setInterval(() => {
      setTask((t) => (t && t.phase === "working" ? { ...t, elapsedS: t.elapsedS + 1 } : t));
    }, 1000);
    const step = window.setInterval(() => {
      setTask((t) => {
        if (!t || t.phase !== "working") return t;
        const next = t.step + 1;
        return next >= TASK_STEPS.length ? { ...t, phase: "done" } : { ...t, step: next };
      });
    }, STEP_MS);
    return () => {
      window.clearInterval(tick);
      window.clearInterval(step);
    };
  }, [phase]);

  return {
    task,
    start: (title: string) => setTask({ title, step: 0, elapsedS: 0, phase: "working" }),
    togglePause: () =>
      setTask((t) => (t && t.phase !== "done" ? { ...t, phase: t.phase === "working" ? "paused" : "working" } : t)),
    stop: () => setTask(null),
  };
}

export function CommandSection() {
  const { transitionStyle } = useFluxGlass();
  const { task, start, togglePause, stop } = useSimulatedTask();

  const detail = task ? (task.phase === "done" ? "Result ready" : TASK_STEPS[task.step]) : undefined;

  return (
    <LabSection
      id="lab-command"
      index="06"
      title="Command → Task"
      lead="The signature surface. Ask, and the same glass becomes the task: one radius, one position, a critically damped spring — or a 180 ms crossfade when motion is reduced or the tier is Minimal."
    >
      <div className="lab-command">
        <div className="lab-command__meta">
          <span className="lab-mono">transition · {transitionStyle}</span>
          <SimulatedTag>Simulated task</SimulatedTag>
        </div>
        <CommandMorph
          mode={task ? "task" : "command"}
          command={<CommandBar onSubmit={start} leading={<PegolesMark size={26} state="idle" decorative />} shortcutHint="⌘K" />}
          task={
            <TaskController
              title={task?.title ?? ""}
              status={STATUS[task?.phase ?? "working"]}
              detail={detail}
              elapsed={task ? formatElapsed(task.elapsedS) : undefined}
              working={task?.phase === "working"}
              leading={<PegolesMark size={28} state={MARK_STATE[task?.phase ?? "working"]} decorative />}
              actions={
                <>
                  {task?.phase !== "done" && (
                    <GlassButton
                      variant="quiet"
                      size="sm"
                      iconOnly
                      icon={task?.phase === "paused" ? <PlayIcon size={12} /> : <PauseIcon size={12} />}
                      onClick={togglePause}
                    >
                      {task?.phase === "paused" ? "Resume task" : "Pause task"}
                    </GlassButton>
                  )}
                  <GlassButton variant="secondary" size="sm" icon={<StopIcon size={12} />} onClick={stop}>
                    {task?.phase === "done" ? "New task" : "Stop"}
                  </GlassButton>
                </>
              }
            />
          }
        />
        <p className="lab-footnote">
          Try: type “Open a terminal and check disk space”, press Enter. Escape clears the field, a second Escape leaves
          it.
        </p>
      </div>
    </LabSection>
  );
}

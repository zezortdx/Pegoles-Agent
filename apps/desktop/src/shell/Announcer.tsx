import type { CopyKey } from "../i18n/catalog";
import { t, useLocale } from "../i18n";
import { useEffect, useRef, useState } from "react";
import type { ComputerPhase } from "../state/computerModel";
import type { AgentTask } from "../lib/tauri";

interface AnnouncerProps {
  readonly tasks: readonly AgentTask[];
  readonly computerPhase: ComputerPhase;
}

const CONTROL_COPY: Partial<Record<ComputerPhase, CopyKey>> = {
  user: "You have control of Pegoles’ computer.",
  agent: "Pegoles has control of its computer.",
  error: "Pegoles’ computer couldn’t start.",
};

/**
 * Screen-reader voice for changes that happen without the person asking:
 * polite for progress, assertive when Pegoles needs them or control moves.
 * Nothing here is conveyed only by motion or color.
 */
export function Announcer({ tasks, computerPhase }: AnnouncerProps) {
  useLocale();
  const [polite, setPolite] = useState("");
  const [urgent, setUrgent] = useState("");
  const previous = useRef<Map<string, AgentTask["status"]> | null>(null);
  const phase = useRef<ComputerPhase | null>(null);

  useEffect(() => {
    const before = previous.current;
    previous.current = new Map(tasks.map((task) => [task.id, task.status]));
    if (!before) return;
    for (const task of tasks) {
      const was = before.get(task.id);
      if (!was || was === task.status) continue;
      if (task.status === "waiting_for_approval") setUrgent(t("Pegoles needs your approval for “{0}”.", { "0": task.title }));
      else if (task.status === "completed") setPolite(t("Pegoles finished “{0}”.", { "0": task.title }));
      else if (task.status === "failed") setUrgent(t("Pegoles couldn’t finish “{0}”.", { "0": task.title }));
      else if (task.status === "running") setPolite(t("Pegoles started “{0}”.", { "0": task.title }));
      else if (task.status === "cancelled" && was === "running") setPolite(t("Pegoles stopped “{0}”.", { "0": task.title }));
    }
  }, [tasks]);

  useEffect(() => {
    const was = phase.current;
    phase.current = computerPhase;
    if (was === null || was === computerPhase) return;
    const copy = CONTROL_COPY[computerPhase];
    if (copy) setUrgent(t(copy));
    else if (computerPhase === "ready" && was === "starting") setPolite(t("Pegoles’ computer is ready."));
  }, [computerPhase]);

  return (
    <>
      <div className="visually-hidden" role="status" aria-live="polite">{polite}</div>
      <div className="visually-hidden" role="alert" aria-live="assertive">{urgent}</div>
    </>
  );
}

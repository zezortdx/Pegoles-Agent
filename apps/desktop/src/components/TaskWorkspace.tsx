import { useMemo } from "react";
import {
  ApprovalCard,
  CommandMorph,
  CurrentAction,
  GlassButton,
  PegolesMark,
  TaskController,
  type StatusTone,
} from "@pegoles/ui";
import { ActivityTimeline } from "./ActivityTimeline";
import { currentActionFor } from "../lib/currentAction";
import type { AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";

export type WorkspaceLayout = "compact" | "split" | "focus";

export interface TaskWorkspaceProps {
  readonly task: AgentTask;
  readonly layout: WorkspaceLayout;
  readonly onLayoutChange: (layout: WorkspaceLayout) => void;
  readonly onBack: () => void;
  readonly headingRef?: React.Ref<HTMLHeadingElement>;
  readonly status: StatusPayload | null;
  readonly connected: boolean;
  readonly events: AgentEvent[];
  readonly taskEvents: AgentEvent[];
  readonly computer: React.ReactNode;
  readonly taskDetail?: string;
  readonly elapsed?: string;
}

function taskTone(status: string): { tone: StatusTone; label: string } {
  switch (status) {
    case "running":
      return { tone: "active", label: "Working" };
    case "waiting_for_approval":
      return { tone: "waiting", label: "Needs approval" };
    case "completed":
      return { tone: "success", label: "Done" };
    case "failed":
      return { tone: "danger", label: "Failed" };
    case "cancelled":
      return { tone: "neutral", label: "Cancelled" };
    default:
      return { tone: "neutral", label: "Pending · execution unavailable" };
  }
}

const LAYOUTS: readonly WorkspaceLayout[] = ["compact", "split", "focus"];
const LAYOUT_LABEL: Record<WorkspaceLayout, string> = { compact: "Compact", split: "Split", focus: "Focus" };

export function TaskWorkspace(props: TaskWorkspaceProps) {
  const { task, layout, onLayoutChange, onBack, headingRef, status, connected, taskEvents, computer } = props;
  const meta = taskTone(task.status);
  const working = task.status === "running" || status?.viewport_state === "agent_active";
  const latestEvent = taskEvents.length ? taskEvents[taskEvents.length - 1] ?? null : null;
  const currentAction = useMemo(
    () => currentActionFor(status, connected, task.status, latestEvent, taskEvents),
    [status, connected, task.status, latestEvent, taskEvents],
  );
  const needsApproval = task.status === "waiting_for_approval";

  return (
    <section className="workspace" data-layout={layout} data-approval={needsApproval ? "true" : undefined}>
      <div className="workspace-heading">
        <div className="workspace-title">
          <GlassButton variant="quiet" size="sm" onClick={onBack}>
            ‹ All tasks
          </GlassButton>
          <h1 ref={headingRef} tabIndex={-1}>
            {task.title}
          </h1>
          <div className="workspace-controller">
            <CommandMorph
              mode="task"
              command={null}
              task={
                <TaskController
                  title={task.title}
                  status={meta}
                  detail={props.taskDetail}
                  elapsed={props.elapsed}
                  working={working}
                  leading={<PegolesMark size={28} state={working ? "acting" : "idle"} decorative />}
                />
              }
            />
          </div>
          {currentAction && (
            <div className="workspace-current">
              <CurrentAction
                label={currentAction.label}
                tone={currentAction.tone}
                presence={currentAction.presence}
                working={currentAction.working}
              />
            </div>
          )}
        </div>
        <div className="layout-switch" role="group" aria-label="Workspace layout">
          {LAYOUTS.map((mode) => (
            <button key={mode} aria-pressed={layout === mode} onClick={() => onLayoutChange(mode)}>
              {LAYOUT_LABEL[mode]}
            </button>
          ))}
        </div>
      </div>
      {needsApproval && (
        <div className="workspace-approval">
          <ApprovalCard
            title={task.title}
            actions={
              <>
                <GlassButton variant="secondary" size="sm" disabled>
                  Review
                </GlassButton>
                <GlassButton variant="primary" size="sm" disabled>
                  Allow
                </GlassButton>
              </>
            }
          />
        </div>
      )}
      <div className="workspace-grid" data-layout={layout}>
        <section
          className="conversation"
          aria-hidden={layout === "focus" ? true : undefined}
          {...(layout === "focus" ? { inert: "" } : {})}
        >
          <h2>Activity</h2>
          <ActivityTimeline events={taskEvents} />
          {task.status === "pending" && (
            <div className="pending-message">
              <PegolesMark size={24} state="idle" decorative />
              <p>Task saved. Autonomous execution is not available yet. You can use Pegoles Computer while this task stays pending.</p>
            </div>
          )}
        </section>
        <div
          className="workspace-computer"
          data-computer-layout={layout}
        >
          {computer}
        </div>
      </div>
    </section>
  );
}

/**
 * Design lab · Task Workspace (Phase 4.1). DEV ONLY — simulated states.
 * Shows idle → active → approval → complete using the real production
 * components (CommandMorph, TaskController, ApprovalCard, CurrentAction,
 * ComputerViewport) with fixture data.
 */
import { useState } from "react";
import {
  ApprovalCard,
  CommandBar,
  CommandMorph,
  ComputerViewport,
  CurrentAction,
  GlassButton,
  PegolesMark,
  TaskController,
  type StatusTone,
} from "@pegoles/ui";
import { LabSection, Segmented, SimulatedTag } from "../lab/controls";
import { stagesFor } from "../fixtures";

type WorkspaceDemo = "idle" | "active" | "approval" | "complete";

const STATUS: Record<WorkspaceDemo, { tone: StatusTone; label: string }> = {
  idle: { tone: "neutral", label: "Ready" },
  active: { tone: "active", label: "Working" },
  approval: { tone: "waiting", label: "Needs approval" },
  complete: { tone: "success", label: "Done" },
};

const ACTION: Record<WorkspaceDemo, string | null> = {
  idle: null,
  active: "Pegoles is opening the browser",
  approval: "Pegoles needs your approval",
  complete: null,
};

export function WorkspaceSection() {
  const [mode, setMode] = useState<WorkspaceDemo>("active");

  return (
    <LabSection
      id="lab-workspace"
      index="13"
      title="Task Workspace"
      lead="Home morphs into a working computer: the command surface becomes the task, activity stays live beside a hero viewport, approvals emerge as glass near the task."
    >
      <div style={{ display: "flex", gap: 12, alignItems: "center", marginBottom: 18, flexWrap: "wrap" }}>
        <Segmented
          label="Workspace state"
          value={mode}
          onChange={setMode}
          size="sm"
          options={[
            { value: "idle", label: "Idle" },
            { value: "active", label: "Active" },
            { value: "approval", label: "Approval" },
            { value: "complete", label: "Complete" },
          ]}
        />
        <SimulatedTag>Simulated task · lab only</SimulatedTag>
      </div>
      <div style={{ display: "grid", gap: 16, maxWidth: 860 }}>
        <CommandMorph
          mode={mode === "idle" ? "command" : "task"}
          command={<CommandBar onSubmit={() => setMode("active")} leading={<PegolesMark size={26} state="idle" decorative />} shortcutHint="⌘K" />}
          task={
            <TaskController
              title="Crie um site sobre viagens"
              status={STATUS[mode]}
              detail={mode === "active" ? "Opening browser" : mode === "complete" ? "Result ready · 2:14" : undefined}
              elapsed={mode === "active" ? "0:42" : mode === "complete" ? "2:14" : undefined}
              working={mode === "active"}
              leading={<PegolesMark size={28} state={mode === "active" ? "acting" : mode === "complete" ? "success" : "idle"} decorative />}
            />
          }
        />
        {ACTION[mode] && (
          <div>
            <CurrentAction
              label={ACTION[mode] ?? ""}
              tone={mode === "approval" ? "waiting" : "active"}
              presence={mode === "approval" ? "waitingForUser" : "acting"}
              working={mode === "active"}
            />
          </div>
        )}
        {mode === "approval" && (
          <ApprovalCard
            title="Apply 4 changes to project"
            detail="+ auth.ts · + middleware.ts · − old-auth.ts (simulated)"
            actions={
              <>
                <GlassButton variant="secondary" size="sm">
                  Review
                </GlassButton>
                <GlassButton variant="primary" size="sm">
                  Allow
                </GlassButton>
              </>
            }
          />
        )}
        <ComputerViewport
          state={mode === "active" ? "agent_active" : mode === "approval" ? "ready" : mode === "complete" ? "ready" : "off"}
          headingLevel={3}
          subtitle="Debian 13 · Weston"
          bootStages={stagesFor(mode === "active" ? "agent_active" : "off")}
          onTakeControl={mode === "active" ? () => undefined : undefined}
          placeholder={
            <div style={{ padding: 28, textAlign: "center", color: "var(--pg-text-secondary)", fontSize: 13 }}>
              {mode === "idle" ? "Pegoles Computer is off" : mode === "complete" ? "Result ready — viewport relaxed" : "Simulated Chromium · lab only"}
            </div>
          }
        />
      </div>
    </LabSection>
  );
}

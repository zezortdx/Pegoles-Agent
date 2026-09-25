import type { ReactNode } from "react";
import { m } from "motion/react";
import { PresenceMark, type PresenceMode } from "../presence";
import type { AgentTask } from "../lib/tauri";
import type { ComputerModel } from "../state/computerModel";
import type { TaskSection } from "../state/taskSections";
import { spring } from "../lib/motion";
import { MachineScreen } from "../computer/MachineScreen";
import { ActivityIcon, ComposeIcon, SearchIcon, SettingsIcon, SidebarIcon } from "../ui/icons";

export type Place = "activity" | "settings";
export type SidebarMode = "shown" | "hidden" | "drawer";

/** A live line under a task that deserves a glance; calm tasks get none. */
export interface TaskGlance {
  readonly text: string;
  readonly tone: "live" | "attention" | "error";
}

export interface SidebarProps {
  readonly sections: readonly TaskSection[];
  readonly glances: Readonly<Record<string, TaskGlance | null>>;
  readonly selectedTaskId: string | null;
  readonly place: Place | null;
  readonly presence: PresenceMode;
  readonly computer: ComputerModel;
  readonly computerOpen: boolean;
  readonly connected: boolean;
  readonly mode: SidebarMode;
  readonly modifier: string;
  readonly onOpenTask: (task: AgentTask) => void;
  readonly onNewTask: () => void;
  readonly onSearch: () => void;
  readonly onPlace: (place: Place) => void;
  readonly onComputer: () => void;
  readonly onToggle: () => void;
}

const SELECTION_ID = "sidebar-selection";

/** The quiet lens behind whichever row is current; it glides between rows. */
function Selection() {
  return <m.span className="nav-row__lens" layoutId={SELECTION_ID} transition={spring.micro} aria-hidden="true" />;
}

function NavRow({ icon, label, selected = false, trailing, onClick, className }: {
  icon: ReactNode; label: string; selected?: boolean; trailing?: ReactNode; onClick: () => void; className?: string;
}) {
  return (
    <button type="button" className={className ? `nav-row ${className}` : "nav-row"} aria-current={selected ? "page" : undefined} onClick={onClick}>
      {selected && <Selection />}
      <span className="nav-row__icon" aria-hidden="true">{icon}</span>
      <span className="nav-row__label">{label}</span>
      {trailing}
    </button>
  );
}

/** What a task row says about its state before you read it. */
function Indicator({ task }: { task: AgentTask }) {
  switch (task.status) {
    case "running": return <span className="task-dot task-dot--live pg-work-anim" aria-hidden="true" />;
    case "waiting_for_approval": return <span className="task-dot task-dot--attention" aria-hidden="true" />;
    case "failed": return <span className="task-dot task-dot--error" aria-hidden="true" />;
    case "pending": return <span className="task-dot task-dot--pending" aria-hidden="true" />;
    default: return <span className="task-dot" aria-hidden="true" />;
  }
}

const STATE_WORD: Partial<Record<AgentTask["status"], string>> = {
  running: "working", waiting_for_approval: "needs you", failed: "couldn’t finish", pending: "not started", cancelled: "cancelled",
};

function TaskRow({ task, glance, selected, onOpen }: { task: AgentTask; glance: TaskGlance | null; selected: boolean; onOpen: () => void }) {
  const word = STATE_WORD[task.status];
  return (
    <li>
      <button type="button" className="task-row" data-status={task.status} data-glance={glance ? true : undefined}
        aria-current={selected ? "page" : undefined} onClick={onOpen} title={task.title}>
        {selected && <Selection />}
        <Indicator task={task} />
        <span className="task-row__text">
          <span className="task-row__title">{task.title}</span>
          {glance && <span className="task-row__glance" data-tone={glance.tone}>{glance.text}</span>}
        </span>
        {word && !glance && <span className="visually-hidden">, {word}</span>}
      </button>
    </li>
  );
}

/**
 * Navigation on a quiet material: who Pegoles is and what it's doing (a
 * small living mark), where to start new work, the work itself grouped by
 * what needs you, and its computer at the foot, like a device you own.
 */
export function Sidebar(props: SidebarProps) {
  const { sections, computer } = props;
  const hidden = props.mode === "hidden";
  const chip = computer.chip;
  return (
    <aside
      className="sidebar"
      data-mode={props.mode}
      aria-label="Sidebar"
      aria-hidden={hidden || undefined}
      {...(hidden ? { inert: "" } : {})}
    >
      <div className="sidebar__chrome" data-tauri-drag-region>
        <button type="button" className="icon-btn" aria-label="Hide sidebar" title={`Hide sidebar (${props.modifier}\\)`} onClick={props.onToggle}>
          <SidebarIcon size={16} />
        </button>
      </div>

      <div className="sidebar__brand" data-tauri-drag-region>
        <span className="sidebar__mark" data-thought-anchor="presence"><PresenceMark mode={props.presence} size={20} decorative={false} /></span>
        <span className="sidebar__name" data-tauri-drag-region>Pegoles</span>
        <button type="button" className="icon-btn sidebar__search" aria-label="Search" title={`Search (${props.modifier}K)`} onClick={props.onSearch}>
          <SearchIcon size={15} />
        </button>
      </div>

      <nav className="sidebar__nav" aria-label="Primary">
        <NavRow icon={<ComposeIcon size={16} />} label="New task" onClick={props.onNewTask} className="nav-row--new"
          trailing={<kbd className="nav-row__kbd">{props.modifier}N</kbd>} />
        <NavRow icon={<ActivityIcon size={16} />} label="Activity" selected={props.place === "activity"} onClick={() => props.onPlace("activity")} />
      </nav>

      <nav className="sidebar__tasks" aria-label="Tasks">
        {sections.length === 0 ? (
          <p className="sidebar__empty">Tasks you hand to Pegoles appear here.</p>
        ) : sections.map((section) => (
          <section key={section.key} className="sidebar__section" data-section={section.key} aria-labelledby={`side-${section.key}`}>
            <h2 id={`side-${section.key}`} className="sidebar__heading">{section.label}</h2>
            <ul className="sidebar__list">
              {section.tasks.map((task) => (
                <TaskRow key={task.id} task={task} glance={props.glances[task.id] ?? null}
                  selected={props.selectedTaskId === task.id} onOpen={() => props.onOpenTask(task)} />
              ))}
            </ul>
          </section>
        ))}
      </nav>

      <div className="sidebar__foot">
        {!props.connected && (
          <p className="sidebar__offline" role="status"><span className="task-dot task-dot--error" aria-hidden="true" />Pegoles Core isn’t connected</p>
        )}
        <button type="button" className="nav-row machine-row" data-phase={computer.phase} aria-pressed={props.computerOpen}
          aria-label={`Pegoles Computer: ${chip}. ${props.computerOpen ? "Hide" : "Show"} its computer`} onClick={props.onComputer}
          data-thought-target="computer">
          <span className="nav-row__icon machine-row__glyph" aria-hidden="true"><MachineScreen model={computer} size="chip" /></span>
          <span className="nav-row__label">Computer</span>
          <span className="machine-row__state" data-phase={computer.phase}>{chip}</span>
        </button>
        <NavRow icon={<SettingsIcon size={16} />} label="Settings" selected={props.place === "settings"} onClick={() => props.onPlace("settings")} />
      </div>
    </aside>
  );
}

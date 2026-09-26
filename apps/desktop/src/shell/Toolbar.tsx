import { AnimatePresence, m } from "motion/react";
import type { ComputerModel } from "../state/computerModel";
import type { StateTone } from "../lib/taskState";
import { duration, ease } from "../lib/motion";
import { ComposeIcon, PanelRightIcon, SidebarIcon } from "../ui/icons";

export interface ToolbarContext {
  readonly title: string;
  readonly tone: StateTone;
  readonly live: boolean;
}

export interface ToolbarProps {
  /** The sidebar is hidden: this bar carries its toggle and New task after the traffic lights. */
  readonly sidebarHidden: boolean;
  /** Task title, shown once the task's own heading has scrolled away. */
  readonly context: ToolbarContext | null;
  readonly computer: ComputerModel;
  readonly computerOpen: boolean;
  /** Hide the Computer toggle (it lives in the panel while the computer is full). */
  readonly showComputerToggle: boolean;
  readonly modifier: string;
  readonly onToggleSidebar: () => void;
  readonly onNewTask: () => void;
  readonly onToggleComputer: () => void;
}

/** Colour of the small light on the Computer toggle: who is acting on it. */
function lightOf(model: ComputerModel): string | undefined {
  switch (model.phase) {
    case "agent": case "starting": return "live";
    case "user": return "attention";
    case "ready": return "ready";
    case "error": return "error";
    default: return undefined;
  }
}

/**
 * The window's top edge over the work: a drag region that holds only what
 * the work column needs — where you are, and the door to its computer.
 */
export function Toolbar(props: ToolbarProps) {
  const { context, computer } = props;
  const light = lightOf(computer);
  return (
    <header className="toolbar" data-sidebar-hidden={props.sidebarHidden || undefined} data-tauri-drag-region>
      {props.sidebarHidden && (
        <div className="toolbar__lead">
          <button type="button" className="icon-btn" aria-label="Show sidebar" title={`Show sidebar (${props.modifier}\\)`} onClick={props.onToggleSidebar}>
            <SidebarIcon size={16} />
          </button>
          <button type="button" className="icon-btn" aria-label="New task" title={`New task (${props.modifier}N)`} onClick={props.onNewTask}>
            <ComposeIcon size={16} />
          </button>
        </div>
      )}
      <div className="toolbar__context" data-tauri-drag-region>
        <AnimatePresence initial={false}>
          {context && (
            <m.p
              key="context"
              className="toolbar__title"
              data-tauri-drag-region
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}
              exit={{ opacity: 0, y: -2, transition: { duration: duration.micro, ease: ease.exit } }}
            >
              <span className="state-dot" data-tone={context.tone} data-live={context.live || undefined} aria-hidden="true" />
              <span className="toolbar__title-text" data-tauri-drag-region>{context.title}</span>
            </m.p>
          )}
        </AnimatePresence>
      </div>
      <div className="toolbar__actions">
        {props.showComputerToggle && (
          <button
            type="button"
            className="icon-btn toolbar__computer"
            aria-pressed={props.computerOpen}
            aria-label={`${props.computerOpen ? "Hide" : "Show"} its computer (${computer.chip})`}
            title={`${props.computerOpen ? "Hide" : "Show"} its computer (${props.modifier}J)`}
            onClick={props.onToggleComputer}
          >
            <PanelRightIcon size={16} />
            {light && <span className="toolbar__light" data-light={light} aria-hidden="true" />}
          </button>
        )}
      </div>
    </header>
  );
}

import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { m } from "motion/react";
import { duration, ease, spring } from "../lib/motion";
import type { AgentTask } from "../lib/tauri";
import { relativeTime } from "../lib/taskPresentation";
import { ActivityIcon, ComposeIcon, ComputerIcon, SearchIcon, SettingsIcon, SidebarIcon } from "../ui/icons";

export interface PaletteCommand {
  readonly id: string;
  readonly label: string;
  readonly icon: ReactNode;
  readonly hint?: string;
  readonly run: () => void;
}

export interface CommandPaletteProps {
  readonly tasks: readonly AgentTask[];
  readonly computerWord: string;
  readonly computerOpen: boolean;
  readonly modifier: string;
  readonly onClose: () => void;
  readonly onNewTask: () => void;
  readonly onOpenTask: (task: AgentTask) => void;
  readonly onComputer: () => void;
  readonly onActivity: () => void;
  readonly onSettings: () => void;
  readonly onToggleSidebar: () => void;
}

interface Entry {
  readonly id: string;
  readonly group: "Actions" | "Tasks";
  readonly label: string;
  readonly icon: ReactNode;
  readonly meta?: string;
  readonly run: () => void;
}

const STATUS_WORD: Partial<Record<AgentTask["status"], string>> = {
  running: "Working", waiting_for_approval: "Needs you", failed: "Couldn’t finish", pending: "Not started",
};

function matches(label: string, needle: string): boolean {
  if (!needle) return true;
  const haystack = label.toLowerCase();
  return needle.split(/\s+/).every((word) => haystack.includes(word));
}

/**
 * ⌘K: everything reachable from the keyboard, on one pane of glass.
 * Actions first, then tasks, filtered as you type.
 */
export function CommandPalette(props: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const listId = useId();
  // Focus moves in on open and returns to where it was on close.
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    input.current?.focus();
    return () => { if (opener?.isConnected) opener.focus({ preventScroll: true }); };
  }, []);

  const { tasks, modifier, computerOpen, computerWord, onClose, onNewTask, onComputer, onActivity, onSettings, onToggleSidebar, onOpenTask } = props;
  const entries = useMemo<Entry[]>(() => {
    const close = (run: () => void) => () => { onClose(); run(); };
    const actions: Entry[] = [
      { id: "new", group: "Actions", label: "New task", icon: <ComposeIcon />, meta: `${modifier}N`, run: close(onNewTask) },
      { id: "computer", group: "Actions", label: computerOpen ? "Hide its computer" : "Show its computer", icon: <ComputerIcon />, meta: `${computerWord} · ${modifier}J`, run: close(onComputer) },
      { id: "activity", group: "Actions", label: "Activity", icon: <ActivityIcon />, run: close(onActivity) },
      { id: "settings", group: "Actions", label: "Settings", icon: <SettingsIcon />, run: close(onSettings) },
      { id: "sidebar", group: "Actions", label: "Toggle sidebar", icon: <SidebarIcon />, meta: `${modifier}\\`, run: close(onToggleSidebar) },
    ];
    const taskEntries: Entry[] = [...tasks]
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      .map((task) => ({
        id: `task-${task.id}`, group: "Tasks", label: task.title, icon: <span className="palette__task-dot" data-status={task.status} />,
        meta: STATUS_WORD[task.status] ?? relativeTime(task.updated_at), run: close(() => onOpenTask(task)),
      }));
    const needle = query.trim().toLowerCase();
    return [...actions, ...taskEntries].filter((entry) => matches(entry.label, needle));
  }, [tasks, modifier, computerOpen, computerWord, onClose, onNewTask, onComputer, onActivity, onSettings, onToggleSidebar, onOpenTask, query]);

  const active = Math.min(index, Math.max(entries.length - 1, 0));
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") { event.preventDefault(); setIndex((value) => (entries.length ? (Math.min(value, entries.length - 1) + 1) % entries.length : 0)); }
    else if (event.key === "ArrowUp") { event.preventDefault(); setIndex((value) => (entries.length ? (Math.min(value, entries.length - 1) - 1 + entries.length) % entries.length : 0)); }
    else if (event.key === "Enter") { event.preventDefault(); entries[active]?.run(); }
    else if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); }
    // A modal: the field is its only stop, so Tab stays here.
    else if (event.key === "Tab") event.preventDefault();
  };

  let lastGroup: Entry["group"] | null = null;
  return (
    <m.div
      className="palette-layer"
      onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1, transition: { duration: duration.microSlow, ease: ease.out } }}
      exit={{ opacity: 0, transition: { duration: duration.micro, ease: ease.exit } }}
    >
      <m.div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        initial={{ opacity: 0, scale: 0.98, y: -6 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.985, transition: { duration: duration.micro, ease: ease.exit } }}
        transition={spring.snappy}
      >
        <div className="palette__field">
          <SearchIcon size={16} />
          <input
            ref={input}
            className="palette__input"
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={entries.length ? `${listId}-${active}` : undefined}
            aria-label="Search tasks and actions"
            placeholder="Search tasks and actions…"
            value={query}
            onChange={(event) => { setQuery(event.target.value); setIndex(0); }}
            onKeyDown={onKey}
            spellCheck={false}
          />
          <kbd className="palette__esc">esc</kbd>
        </div>
        <ul ref={list} id={listId} className="palette__list" role="listbox" aria-label="Results">
          {entries.length === 0 && <li className="palette__empty" role="presentation">Nothing matches “{query.trim()}”.</li>}
          {entries.map((entry, position) => {
            const heading = entry.group !== lastGroup ? entry.group : null;
            lastGroup = entry.group;
            return [
              heading && <li key={`h-${heading}`} className="palette__group" role="presentation">{heading}</li>,
              <li
                key={entry.id}
                id={`${listId}-${position}`}
                data-index={position}
                role="option"
                aria-selected={position === active}
                className="palette__option"
                onMouseMove={() => setIndex(position)}
                onClick={entry.run}
              >
                {position === active && <m.span className="palette__capsule" layoutId="palette-capsule" transition={spring.snappy} aria-hidden="true" />}
                <span className="palette__icon">{entry.icon}</span>
                <span className="palette__label">{entry.label}</span>
                {entry.meta && <span className="palette__meta">{entry.meta}</span>}
              </li>,
            ];
          })}
        </ul>
      </m.div>
    </m.div>
  );
}

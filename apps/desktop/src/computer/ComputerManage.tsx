import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import type { ComputerCommand } from "../state/computerModel";

export type ManageCommand = Extract<ComputerCommand, "reset" | "remove">;

const CONFIRM: Record<ManageCommand, { readonly question: string; readonly body: string; readonly action: string }> = {
  reset: {
    question: "Reset its computer?",
    body: "Everything done inside it is erased: its apps, files and settings go back to the sealed Pegoles image.",
    action: "Reset",
  },
  remove: {
    question: "Remove its computer?",
    body: "Its disk and everything on it are deleted. Pegoles creates a fresh one from its image the next time it needs it.",
    action: "Remove",
  },
};

export interface ComputerManageProps {
  /** The computer exists (there is something to reset or remove). */
  readonly available: boolean;
  /** A task is running on it: Core refuses both until it stops. */
  readonly locked: boolean;
  readonly busy: boolean;
  readonly onCommand: (command: ManageCommand) => void;
}

/**
 * The two ways to start over, kept out of the way and never one click
 * from happening: each asks inline first (no modal, nothing blocks the
 * window) with Cancel focused, and both wait while a task is running.
 */
export function ComputerManage({ available, locked, busy, onCommand }: ComputerManageProps) {
  const [confirming, setConfirming] = useState<ManageCommand | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const triggers = useRef<Partial<Record<ManageCommand, HTMLButtonElement | null>>>({});
  const restore = useRef<ManageCommand | null>(null);
  const titleId = useId();
  // A run that starts meanwhile closes the question: nothing can be confirmed under it.
  const open = locked ? null : confirming;

  useEffect(() => {
    if (open) { cancelRef.current?.focus(); return; }
    const back = restore.current;
    restore.current = null;
    if (back) triggers.current[back]?.focus();
  }, [open]);

  if (!available) return null;

  const dismiss = () => { restore.current = confirming; setConfirming(null); };
  if (open) {
    const copy = CONFIRM[open];
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      dismiss();
    };
    return (
      <div className="manage manage--confirm" role="group" aria-labelledby={titleId} onKeyDown={onKeyDown}>
        <p id={titleId} className="manage__question">{copy.question}</p>
        <p className="manage__body">{copy.body}</p>
        <div className="manage__actions">
          <button ref={cancelRef} type="button" className="btn btn--quiet btn--small" onClick={dismiss}>Cancel</button>
          <button type="button" className="btn btn--danger btn--small" disabled={busy}
            onClick={() => { onCommand(open); setConfirming(null); }}>
            {copy.action}
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="manage">
      <div className="manage__actions" role="group" aria-label="Start its computer over">
        <button ref={(element) => { triggers.current.reset = element; }} type="button" className="btn btn--quiet btn--small"
          disabled={locked || busy} onClick={() => setConfirming("reset")}>
          Reset computer…
        </button>
        <button ref={(element) => { triggers.current.remove = element; }} type="button" className="btn btn--quiet btn--small manage__remove"
          disabled={locked || busy} onClick={() => setConfirming("remove")}>
          Remove computer…
        </button>
      </div>
      {locked && <p className="manage__why">Stop the running task to reset or remove its computer.</p>}
    </div>
  );
}

import { m } from "motion/react";
import type { ComputerModel } from "../state/computerModel";
import { duration, ease, spring } from "../lib/motion";
import { CloseIcon, ExpandIcon } from "../ui/icons";
import { MachineScreen } from "./MachineScreen";
import type { Snapshot } from "./useScreenSnapshot";

export interface ComputerPeekProps {
  readonly model: ComputerModel;
  readonly snapshot: Snapshot | null;
  /** What Pegoles is doing on it, in a few words. */
  readonly caption: string;
  readonly onOpen: () => void;
  readonly onDismiss: () => void;
}

/**
 * The computer's compact level: a small live surface in the corner of the
 * work while Pegoles uses it and its panel is closed. It takes no room
 * from the task; one click gives it a column.
 */
export function ComputerPeek({ model, snapshot, caption, onOpen, onDismiss }: ComputerPeekProps) {
  return (
    <m.div
      className="peek"
      initial={{ opacity: 0, y: 14, scale: 0.94 }}
      animate={{ opacity: 1, y: 0, scale: 1, transition: spring.surface }}
      exit={{ opacity: 0, y: 8, scale: 0.97, transition: { duration: duration.micro, ease: ease.exit } }}
    >
      <button type="button" className="peek__screen" onClick={onOpen} aria-label={`Watch its computer: ${caption}`}>
        {snapshot?.src
          ? <img className="peek__image" src={snapshot.src} alt="" draggable={false} />
          : <span className="peek__empty"><MachineScreen model={model} size="card" /></span>}
        <span className="peek__open" aria-hidden="true"><ExpandIcon size={13} /></span>
      </button>
      <div className="peek__caption">
        <span className="state-dot" data-tone="live" data-live aria-hidden="true" />
        <span className="peek__text">{caption}</span>
        <button type="button" className="icon-btn peek__close" aria-label="Hide preview" onClick={onDismiss}>
          <CloseIcon size={12} />
        </button>
      </div>
    </m.div>
  );
}

import { useMemo } from "react";
import { m } from "motion/react";
import { AgentCursorOverlay } from "@pegoles/ui";
import type { ComputerModel } from "../state/computerModel";
import { duration, ease, spring } from "../lib/motion";
import { CloseIcon, ExpandIcon } from "../ui/icons";
import { useCursorSource, useFrameSize } from "./useAgentCursor";
import { MachineScreen } from "./MachineScreen";
import type { Snapshot } from "./useScreenSnapshot";

export interface ComputerPeekProps {
  readonly model: ComputerModel;
  readonly snapshot: Snapshot | null;
  /** The computer whose actions the agent cursor shows. */
  readonly computerId: string | null;
  /** Guest framebuffer size, until the picture reports its own. */
  readonly display: { readonly width_px: number; readonly height_px: number } | null;
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
export function ComputerPeek({ model, snapshot, computerId, display, caption, onOpen, onDismiss }: ComputerPeekProps) {
  const displayWidth = display?.width_px;
  const displayHeight = display?.height_px;
  const displaySize = useMemo(() => (displayWidth && displayHeight ? { width: displayWidth, height: displayHeight } : null), [displayWidth, displayHeight]);
  const { frameSize, onLoad } = useFrameSize(displaySize);
  const cursorSource = useCursorSource(computerId);
  return (
    <m.div
      className="peek"
      initial={{ opacity: 0, y: 14, scale: 0.94 }}
      animate={{ opacity: 1, y: 0, scale: 1, transition: spring.surface }}
      exit={{ opacity: 0, y: 8, scale: 0.97, transition: { duration: duration.micro, ease: ease.exit } }}
    >
      <button type="button" className="peek__screen" onClick={onOpen} aria-label={`Watch its computer: ${caption}`}>
        {snapshot?.src
          ? <img className="peek__image" src={snapshot.src} alt="" draggable={false} onLoad={onLoad} />
          : <span className="peek__empty"><MachineScreen model={model} size="card" /></span>}
        {snapshot?.src && cursorSource && (
          <AgentCursorOverlay key={computerId} source={cursorSource} frameSize={frameSize} fit="cover" className="peek__cursor" />
        )}
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

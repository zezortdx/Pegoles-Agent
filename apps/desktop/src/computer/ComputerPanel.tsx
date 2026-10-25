import { useEffect, useMemo, useRef, type CSSProperties } from "react";
import { AgentCursorOverlay } from "@pegoles/ui";
import type { ComputerCommand, ComputerModel } from "../state/computerModel";
import type { ActionStep } from "../state/agentState";
import type { HumanError } from "../state/errors";
import type { BootLogPayload, StatusPayload } from "../lib/tauri";
import { timeOf } from "../artifacts/format";
import { CapabilityGlyph } from "../artifacts/glyphs";
import { ArrowLeftIcon, CloseIcon, ExitFullscreenIcon, FocusIcon, FullscreenIcon, UnfocusIcon } from "../ui/icons";
import { ComputerManage, type ManageCommand } from "./ComputerManage";
import { ComputerStatus } from "./ComputerStatus";
import { ControlStrip } from "./ControlStrip";
import { Details, Facts } from "./ComputerInfo";
import { machineLight } from "./MachineScreen";
import { NativeSlot } from "./NativeSlot";
import { useCursorSource, useFrameSize } from "./useAgentCursor";
import type { ComputerLevel } from "./layout";
import type { Snapshot } from "./useScreenSnapshot";
import { useNow } from "../shell/useNow";

export const COMPUTER_PANEL_ID = "pegoles-computer";
const DEFAULT_ASPECT = 16 / 10;

export interface ComputerPanelProps {
  readonly model: ComputerModel;
  readonly level: ComputerLevel;
  readonly status: StatusPayload | null;
  /** A native framebuffer view may attach (desktop app, real backend, display available). */
  readonly slotEnabled: boolean;
  /** A real picture of the screen when there is no live view. */
  readonly snapshot: Snapshot | null;
  /** Recent steps of the task in view that touched this computer. */
  readonly steps: readonly ActionStep[];
  readonly busy: boolean;
  /** Reset or remove is in flight. */
  readonly managing: boolean;
  readonly error: HumanError | null;
  /** The workspace is recomposing: keep the native view hidden until it settles. */
  readonly moving: boolean;
  /** Covered by other UI (drawer, palette): hide the native view without detaching. */
  readonly obscured?: boolean;
  /** Opened by the person: focus moves in. Opened by Pegoles: never steal focus. */
  readonly focusOnOpen: boolean;
  readonly onCommand: (command: ComputerCommand) => void;
  readonly onManage: (command: ManageCommand) => void;
  readonly onLevel: (level: ComputerLevel) => void;
  readonly onClose: () => void;
  readonly onSlotError: (error: unknown) => void;
  readonly onDismissError: () => void;
  readonly loadBootLog: () => Promise<BootLogPayload>;
  readonly style?: CSSProperties;
}

function stateTone(model: ComputerModel): string {
  switch (model.phase) {
    case "agent": case "starting": return "live";
    case "user": return "attention";
    case "error": return "error";
    case "ready": return "done";
    default: return "quiet";
  }
}

function ageOf(at: number | null, now: number): string | null {
  if (!at) return null;
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  return seconds < 5 ? "just now" : seconds < 60 ? `${seconds}s ago` : `${Math.floor(seconds / 60)}m ago`;
}

/** Running, with no live view and no picture yet: the same frame, said plainly. */
function ScreenPlaceholder({ model, snapshot }: { model: ComputerModel; snapshot: Snapshot | null }) {
  const paused = model.phase === "paused";
  const title = paused ? "Paused" : snapshot?.loading ? "Looking at its screen…" : "Its screen isn’t available";
  const body = paused ? "Its apps and files are kept exactly as they were."
    : snapshot?.error ? "Pegoles couldn’t capture its screen just now. Everything it does still appears in the task."
      : "Everything Pegoles does on it still appears in the task.";
  return (
    <div className="screen-placeholder" data-paused={paused || undefined}>
      <p className="screen-placeholder__title">{title}</p>
      <p className="screen-placeholder__body">{body}</p>
    </div>
  );
}

/** A workspace path reads as its file name; the full path stays in the tooltip. */
function shortPath(detail: string): string {
  return detail.startsWith("/") && !detail.includes(" ") ? detail.split("/").filter(Boolean).pop() ?? detail : detail;
}

function StepsHere({ steps }: { steps: readonly ActionStep[] }) {
  if (!steps.length) return null;
  return (
    <section className="computer__section" aria-labelledby="computer-steps">
      <h3 id="computer-steps" className="computer__section-title">Latest on this computer</h3>
      <ol className="here">
        {steps.map((step) => (
          <li key={step.id} className="here__row" data-outcome={step.outcome}>
            <span className="here__icon" aria-hidden="true"><CapabilityGlyph capability={step.capability} size={13} /></span>
            <span className="here__label">{step.label}</span>
            {step.detail && <span className="here__detail" title={step.detail}>{shortPath(step.detail)}</span>}
            <time className="here__time" dateTime={step.at}>{step.outcome === "running" ? "now" : timeOf(step.at)}</time>
          </li>
        ))}
      </ol>
    </section>
  );
}

/**
 * Pegoles Computer: one mounted object for Side, Focus and Full, so the
 * native slot survives every level change and a person's control session
 * never detaches. The frame is the same in every state: dark when off,
 * lit through the real boot stages, then the live screen (or a real
 * snapshot of it). Nothing on the path from <aside> to the slot
 * transforms or filters.
 */
export function ComputerPanel(props: ComputerPanelProps) {
  const { model, level, status, busy, error, onCommand, snapshot } = props;
  const heading = useRef<HTMLHeadingElement>(null);
  const full = level === "full";
  useEffect(() => {
    if (props.focusOnOpen) heading.current?.focus({ preventScroll: true });
  }, [level, props.focusOnOpen]);

  const screenUp = model.running || model.phase === "paused";
  const display = status?.display_config ?? null;
  const aspect = display ? display.width_px / display.height_px : DEFAULT_ASPECT;
  const locked = model.owner === "user";
  const showSnapshot = screenUp && !props.slotEnabled && !!snapshot?.src;
  const style = { ...props.style, "--screen-ar": aspect } as CSSProperties;
  const now = useNow(showSnapshot, 5000);
  const displayWidth = display?.width_px;
  const displayHeight = display?.height_px;
  const displaySize = useMemo(() => (displayWidth && displayHeight ? { width: displayWidth, height: displayHeight } : null), [displayWidth, displayHeight]);
  const { frameSize, onLoad } = useFrameSize(displaySize);
  const computerId = status?.computer_id ?? null;
  const cursorSource = useCursorSource(computerId);

  return (
    <aside id={COMPUTER_PANEL_ID} className="computer" data-level={level} data-phase={model.phase} data-owner={model.owner}
      data-screen={screenUp || undefined} aria-labelledby="computer-title" style={style}>
      <div className="computer__inner">
        <header className="computer__head" data-tauri-drag-region>
          {full && (
            <button type="button" className="btn btn--quiet btn--small computer__back" onClick={() => props.onLevel("focus")} title="Back to the task (Esc)">
              <ArrowLeftIcon size={13} />Task
            </button>
          )}
          <h2 id="computer-title" ref={heading} tabIndex={-1} className="computer__title">Computer</h2>
          <span className="state-label computer__state" data-tone={stateTone(model)}>
            <span className="state-dot" data-tone={stateTone(model)} data-live={model.transitioning || model.phase === "agent" || undefined} aria-hidden="true" />
            {model.chip}
          </span>
          <span className="computer__head-space" data-tauri-drag-region />
          {screenUp && !full && (
            <button type="button" className="icon-btn" aria-label={level === "focus" ? "Show beside the task" : "Focus its computer"}
              title={level === "focus" ? "Beside the task" : "Focus"} onClick={() => props.onLevel(level === "focus" ? "side" : "focus")}>
              {level === "focus" ? <UnfocusIcon size={16} /> : <FocusIcon size={16} />}
            </button>
          )}
          {screenUp && (
            <button type="button" className="icon-btn" aria-label={full ? "Leave full window" : "Fill the window"}
              title={full ? "Leave full window (Esc)" : "Fill the window"} onClick={() => props.onLevel(full ? "focus" : "full")}>
              {full ? <ExitFullscreenIcon size={15} /> : <FullscreenIcon size={15} />}
            </button>
          )}
          {!full && (
            <button type="button" className="icon-btn" aria-label="Close computer" title={locked ? "Give control back first" : "Close (Esc)"} disabled={locked} onClick={props.onClose}>
              <CloseIcon size={15} />
            </button>
          )}
        </header>

        <div className="computer__body">
          <div className="computer__stage">
            <div className="screen-frame" data-light={machineLight(model)}>
              <div className="computer__screen" data-owner={model.owner}>
                {screenUp ? (
                  <>
                    <NativeSlot enabled={props.slotEnabled} obscured={props.obscured || props.moving} onError={props.onSlotError} display={display}>
                      {showSnapshot
                        ? <img className="screen-snapshot" src={snapshot?.src ?? undefined} alt="Latest snapshot of Pegoles’ computer screen" draggable={false} onLoad={onLoad} />
                        : <ScreenPlaceholder model={model} snapshot={snapshot} />}
                    </NativeSlot>
                    {/* The agent's cursor, over the picture only (never baked into it): real actions, mapped into the drawn frame. */}
                    {showSnapshot && cursorSource && <AgentCursorOverlay key={computerId} source={cursorSource} frameSize={frameSize} />}
                  </>
                ) : (
                  <ComputerStatus model={model} busy={busy} error={error} onCommand={onCommand} onDismissError={props.onDismissError} />
                )}
              </div>
            </div>
            {screenUp && error && (
              <p className="computer__inline-error" role="alert">
                {error.title} <button type="button" className="link-btn" onClick={props.onDismissError}>Dismiss</button>
              </p>
            )}
            {screenUp && (
              <ControlStrip model={model} busy={busy} onCommand={onCommand} canTake={props.slotEnabled}
                snapshotAge={showSnapshot ? ageOf(snapshot?.at ?? null, Math.max(now, snapshot?.at ?? 0)) : null} onRefreshSnapshot={snapshot?.refresh}
                compact={level === "side"} />
            )}
          </div>

          {!full && <StepsHere steps={props.steps} />}
          {level === "side" && <Facts model={model} />}
          {level === "side" && (
            <footer className="computer__foot">
              <ComputerManage available={!!status?.computer_created} locked={!!status?.active_task}
                busy={busy || props.managing} onCommand={props.onManage} />
              <Details model={model} status={status} error={error} loadBootLog={props.loadBootLog} />
            </footer>
          )}
        </div>
      </div>
    </aside>
  );
}

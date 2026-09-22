import { createElement, useId, type CSSProperties, type ReactNode, type Ref } from "react";
import { GlassButton } from "./GlassButton.js";
import { AlertIcon, PauseIcon, PointerIcon, PowerIcon, ReturnIcon } from "./icons.js";
import { ProgressLine } from "./ProgressLine.js";
import { StatusIndicator } from "./StatusIndicator.js";
import {
  BOOT_STAGE_STATUS_TEXT,
  currentBootStage,
  DEFAULT_DISPLAY,
  displayAspect,
  VIEWPORT_STATE_SPECS,
  type BootStage,
  type DisplaySize,
  type ViewportState,
} from "./viewportModel.js";
import { cx } from "./cx.js";
import "./viewport.css";

export interface ComputerViewportProps {
  /** Core-derived state (never guessed by the UI). */
  readonly state: ViewportState;
  /** Guest framebuffer size; the slot keeps this aspect ratio. */
  readonly display?: DisplaySize;
  /**
   * A native framebuffer view sits over the slot. The slot is then an
   * empty hole (outline + radius only) and nothing renders inside it.
   */
  readonly nativeSurface?: boolean;
  /** Ref callback on the framebuffer slot element (geometry bridge). */
  readonly slotRef?: (element: HTMLDivElement | null) => void;
  readonly title?: string;
  /** e.g. "Debian 13 · Weston". Display size is appended automatically. */
  readonly subtitle?: string;
  readonly headingLevel?: 2 | 3 | 4;
  /** Real startup stages (see BootStage). */
  readonly bootStages?: readonly BootStage[];
  /** Real detail next to the status, e.g. "Ready in 4.2 s". */
  readonly statusDetail?: string;
  readonly errorMessage?: string;
  /** Recovery actions for the error state (GlassButtons). */
  readonly errorActions?: ReactNode;
  /** Actions for the off state, e.g. a Start button. */
  readonly offActions?: ReactNode;
  /** Header actions (Pause / Stop …). */
  readonly headerActions?: ReactNode;
  /** Provided → "Take control" is offered when the state allows it. */
  readonly onTakeControl?: () => void;
  /** Provided → "Return to Pegoles" is offered while the user controls. */
  readonly onReturnControl?: () => void;
  /** Attached to whichever control button is rendered (focus management). */
  readonly controlButtonRef?: Ref<HTMLButtonElement>;
  /** Native escape shortcut hint. */
  readonly returnShortcut?: string;
  /** Slot content when no native surface is present (design lab, headless). */
  readonly placeholder?: ReactNode;
  readonly variant?: "full" | "compact";
  /**
   * `width` (default): full width, height from the aspect ratio.
   * `contain`: fit inside a parent with a definite height.
   */
  readonly fit?: "width" | "contain";
  readonly className?: string;
}

function StageGlyph({ status }: { readonly status: BootStage["status"] }) {
  return (
    <svg className="pg-stage__glyph" data-status={status} viewBox="0 0 12 12" aria-hidden="true" focusable="false">
      {status === "done" && (
        <path d="m2.6 6.3 2.2 2.2 4.6-5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
      )}
      {status === "active" && (
        <path
          d="M4.6 1.5h2.8a1 1 0 0 1 .7.3l2.1 2.1a1 1 0 0 1 .3.7v2.8a1 1 0 0 1-.3.7l-2.1 2.1a1 1 0 0 1-.7.3H4.6a1 1 0 0 1-.7-.3L1.8 8.1a1 1 0 0 1-.3-.7V4.6a1 1 0 0 1 .3-.7l2.1-2.1a1 1 0 0 1 .7-.3Z"
          fill="currentColor"
        />
      )}
      {status === "pending" && <circle cx="6" cy="6" r="3.6" fill="none" stroke="currentColor" strokeWidth="1.3" />}
      {status === "failed" && (
        <rect x="2.8" y="2.8" width="6.4" height="6.4" rx="1" transform="rotate(45 6 6)" fill="currentColor" />
      )}
      {status === "skipped" && <path d="M3 6h6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />}
    </svg>
  );
}

function BootStageList({ stages }: { readonly stages: readonly BootStage[] }) {
  return (
    <ol className="pg-stages" aria-label="Startup stages">
      {stages.map((stage) => {
        const hasProgress = typeof stage.progress === "number";
        return (
          <li key={stage.id} className="pg-stage" data-status={stage.status}>
            <StageGlyph status={stage.status} />
            <span className="pg-stage__label">
              {stage.label}
              <span className="pg-visually-hidden">, {BOOT_STAGE_STATUS_TEXT[stage.status]}</span>
            </span>
            {stage.detail && <span className="pg-stage__detail">{stage.detail}</span>}
            {stage.status === "active" && (
              <ProgressLine
                className="pg-stage__progress"
                value={hasProgress ? (stage.progress ?? null) : null}
                showValue={hasProgress}
                label={stage.label}
              />
            )}
          </li>
        );
      })}
    </ol>
  );
}

function CompactStage({ stage }: { readonly stage: BootStage }) {
  const hasProgress = typeof stage.progress === "number";
  return (
    <div className="pg-compact-stage" data-status={stage.status}>
      <span className="pg-compact-stage__label">
        {stage.label}
        {stage.detail && <span className="pg-stage__detail"> · {stage.detail}</span>}
      </span>
      {stage.status === "active" && (
        <ProgressLine value={hasProgress ? (stage.progress ?? null) : null} showValue={hasProgress} label={stage.label} />
      )}
    </div>
  );
}

function SlotContent(props: {
  readonly state: ViewportState;
  readonly compact: boolean;
  readonly display: DisplaySize;
  readonly stages: readonly BootStage[];
  readonly errorMessage?: string;
  readonly errorActions?: ReactNode;
  readonly offActions?: ReactNode;
  readonly placeholder?: ReactNode;
}) {
  const { state, compact, display, stages } = props;
  const spec = VIEWPORT_STATE_SPECS[state];

  if (state === "off") {
    return (
      <div className="pg-slot-panel" data-panel="off">
        <PowerIcon size={compact ? 16 : 20} className="pg-slot-panel__icon" />
        <p className="pg-slot-panel__title">Pegoles Computer is off</p>
        {!compact && <p className="pg-slot-panel__body">Its screen appears here once it starts.</p>}
        {props.offActions && <div className="pg-slot-panel__actions">{props.offActions}</div>}
      </div>
    );
  }

  if (spec.booting) {
    const current = currentBootStage(stages);
    return (
      <div className="pg-slot-panel" data-panel="boot">
        <p className="pg-slot-panel__eyebrow">{spec.label}</p>
        {!compact && <p className="pg-slot-panel__title">{spec.announcement.replace(/\.$/, "")}</p>}
        {stages.length > 0 &&
          (compact ? current && <CompactStage stage={current} /> : <BootStageList stages={stages} />)}
        {stages.length === 0 && <ProgressLine value={null} label={spec.label} className="pg-slot-panel__progress" />}
      </div>
    );
  }

  if (state === "paused") {
    return (
      <div className="pg-slot-panel" data-panel="paused">
        <PauseIcon size={compact ? 16 : 20} className="pg-slot-panel__icon" />
        <p className="pg-slot-panel__title">Paused</p>
        {!compact && <p className="pg-slot-panel__body">Memory is kept; nothing runs until you resume.</p>}
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="pg-slot-panel" data-panel="error">
        <AlertIcon size={compact ? 16 : 20} className="pg-slot-panel__icon" />
        <p className="pg-slot-panel__title">Pegoles Computer hit a problem</p>
        {props.errorMessage && (
          <p className="pg-slot-panel__body" role="alert">
            {props.errorMessage}
          </p>
        )}
        {props.errorActions && <div className="pg-slot-panel__actions">{props.errorActions}</div>}
      </div>
    );
  }

  if (props.placeholder !== undefined) return <>{props.placeholder}</>;
  return (
    <div className="pg-slot-panel" data-panel="no-display">
      <p className="pg-slot-panel__dims">
        {display.width_px} × {display.height_px}
      </p>
      <p className="pg-slot-panel__body">Display not attached</p>
    </div>
  );
}

/**
 * Presentational shell for Pegoles' own computer. Renders chrome AROUND
 * a framebuffer slot that keeps the guest display's aspect ratio; the
 * native view (macOS) is positioned over the slot by the geometry
 * bridge, so with `nativeSurface` nothing is ever drawn inside it.
 */
export function ComputerViewport({
  state,
  display = DEFAULT_DISPLAY,
  nativeSurface = false,
  slotRef,
  title = "Pegoles Computer",
  subtitle,
  headingLevel = 2,
  bootStages = [],
  statusDetail,
  errorMessage,
  errorActions,
  offActions,
  headerActions,
  onTakeControl,
  onReturnControl,
  controlButtonRef,
  returnShortcut = "⌃⌥⎋",
  placeholder,
  variant = "full",
  fit = "width",
  className,
}: ComputerViewportProps) {
  const spec = VIEWPORT_STATE_SPECS[state];
  const headingId = useId();
  const compact = variant === "compact";
  const aspect = displayAspect(display);
  const frameStyle = {
    aspectRatio: `${display.width_px} / ${display.height_px}`,
    "--pg-display-aspect": String(aspect),
  } as CSSProperties;
  const meta = [subtitle, `${display.width_px} × ${display.height_px}`].filter(Boolean).join(" · ");
  const current = currentBootStage(bootStages);

  const showTakeControl = spec.canTakeControl && onTakeControl !== undefined;
  const showReturn = state === "user_controlled" && onReturnControl !== undefined;
  const showNativeBoot = nativeSurface && spec.booting && current !== null;
  const showNativeError = nativeSurface && state === "error" && errorMessage !== undefined;
  const hasFooter = showTakeControl || showReturn || showNativeBoot || showNativeError;

  return (
    <section
      className={cx("pg-viewport", className)}
      data-state={state}
      data-energy={spec.energy}
      data-variant={variant}
      data-fit={fit}
      data-native={nativeSurface ? "true" : "false"}
      aria-labelledby={headingId}
    >
      <header className="pg-viewport__bar">
        <div className="pg-viewport__identity">
          {createElement(`h${headingLevel}`, { id: headingId, className: "pg-viewport__title" }, title)}
          {!compact && <p className="pg-viewport__meta">{meta}</p>}
        </div>
        <StatusIndicator
          className="pg-viewport__status"
          tone={spec.tone}
          label={spec.label}
          detail={statusDetail}
          pulse={state === "agent_active"}
          size={compact ? "sm" : "md"}
          pill
        />
        {headerActions && <div className="pg-viewport__actions">{headerActions}</div>}
      </header>

      <div className="pg-viewport__stage">
        <div className="pg-viewport__frame" style={frameStyle}>
          <div ref={slotRef} className="pg-viewport__slot" data-framebuffer-slot="">
            {!nativeSurface && (
              <SlotContent
                state={state}
                compact={compact}
                display={display}
                stages={bootStages}
                errorMessage={errorMessage}
                errorActions={errorActions}
                offActions={offActions}
                placeholder={placeholder}
              />
            )}
          </div>
          <span className="pg-viewport__edge" data-layer="rest" aria-hidden="true" />
          <span className="pg-viewport__edge" data-layer="energy" aria-hidden="true" />
          {state === "agent_active" && (
            <span className="pg-viewport__edge pg-ambient" data-layer="breath" aria-hidden="true" />
          )}
          <span className="pg-viewport__edge" data-layer="user" aria-hidden="true" />
          <span className="pg-viewport__edge" data-layer="error" aria-hidden="true" />
        </div>
      </div>

      {hasFooter && (
        <footer className="pg-viewport__footer">
          {showNativeBoot && current && <CompactStage stage={current} />}
          {showNativeError && (
            <div className="pg-viewport__error" role="alert">
              <AlertIcon size={14} />
              <span>{errorMessage}</span>
              {errorActions}
            </div>
          )}
          {showTakeControl && (
            <div className="pg-viewport__control">
              <GlassButton
                ref={controlButtonRef}
                variant="secondary"
                size={compact ? "sm" : "md"}
                icon={<PointerIcon size={14} />}
                onClick={onTakeControl}
              >
                Take control
              </GlassButton>
              {!compact && (
                <span className="pg-viewport__hint">Use your own keyboard and pointer inside Pegoles Computer.</span>
              )}
            </div>
          )}
          {showReturn && (
            <div className="pg-viewport__control" data-control="user">
              <StatusIndicator
                tone="user"
                label={compact ? VIEWPORT_STATE_SPECS.user_controlled.label : "You're controlling Pegoles Computer"}
                size={compact ? "sm" : "md"}
              />
              {!compact && (
                <span className="pg-viewport__hint">
                  Press <kbd className="pg-kbd">{returnShortcut}</kbd> or
                </span>
              )}
              <GlassButton
                ref={controlButtonRef}
                variant="primary"
                tone="neutral"
                size={compact ? "sm" : "md"}
                icon={<ReturnIcon size={14} />}
                onClick={onReturnControl}
              >
                Return to Pegoles
              </GlassButton>
            </div>
          )}
        </footer>
      )}

      <p className="pg-visually-hidden" role="status" aria-live="polite">
        {spec.announcement}
      </p>
    </section>
  );
}

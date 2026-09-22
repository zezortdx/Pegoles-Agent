import type { ReactNode } from "react";
import { useGlassSurface } from "./GlassSurface.js";
import { StatusIndicator, type StatusTone } from "./StatusIndicator.js";
import { cx } from "./cx.js";
import "./approval.css";

export interface ApprovalCardProps {
  /** Real task title awaiting approval. */
  readonly title: string;
  /** Real status label, e.g. "Waiting for approval". */
  readonly statusLabel?: string;
  readonly statusTone?: StatusTone;
  /** Real detail when Phase 5 provides it; otherwise honest placeholder copy. */
  readonly detail?: string;
  /** Actions slot (Review / Allow). Disabled until backend provides payload. */
  readonly actions?: ReactNode;
  readonly className?: string;
}

/**
 * Approval surface: glass that emerges near the task, never a modal.
 * Production renders only when Core reports `waiting_for_approval`;
 * file lists arrive with the Phase 5 approval payload.
 */
export function ApprovalCard({
  title,
  statusLabel = "Needs your approval",
  statusTone = "waiting",
  detail,
  actions,
  className,
}: ApprovalCardProps) {
  const glass = useGlassSurface("electric", "ApprovalCard");
  return (
    <section
      ref={glass.ref}
      className={cx("pg-glass", "pg-approval", className)}
      {...glass.attributes}
      data-elevation="3"
      data-radius="lg"
      aria-label="Pegoles needs permission"
      aria-live="polite"
    >
      <div className="pg-approval__head">
        <StatusIndicator tone={statusTone} label={statusLabel} size="sm" live pulse={statusTone === "waiting"} pill />
      </div>
      <p className="pg-approval__title">{title}</p>
      <p className="pg-approval__detail">
        {detail ?? "Pegoles paused before changing anything. Details arrive with the approval payload."}
      </p>
      {actions && <div className="pg-approval__actions">{actions}</div>}
    </section>
  );
}

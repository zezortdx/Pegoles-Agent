import { PegolesMark } from "../presence/PegolesMark.js";
import type { PresenceState } from "../presence/presenceMachine.js";
import { StatusIndicator, type StatusTone } from "./StatusIndicator.js";
import { cx } from "./cx.js";
import "./currentAction.css";

export interface CurrentActionProps {
  /** Compact contextual label, e.g. "Pegoles is typing". Real state only. */
  readonly label: string;
  readonly tone?: StatusTone;
  readonly presence?: PresenceState;
  /** True while real work is in flight: logo-light pulse replaces spinners. */
  readonly working?: boolean;
  readonly className?: string;
}

/**
 * Current action indicator: a compact, contextual label driven by real
 * Core state. The working light comes from the Pegoles mark language —
 * a small electric highlight, never a generic spinner.
 */
export function CurrentAction({ label, tone = "active", presence = "acting", working = true, className }: CurrentActionProps) {
  return (
    <div
      className={cx("pg-current-action", className)}
      data-tone={tone}
      data-working={working ? "true" : "false"}
      role="status"
      aria-live="polite"
    >
      <span className="pg-current-action__mark" aria-hidden="true">
        <PegolesMark size={18} state={presence} decorative />
        {working && <span className="pg-current-action__light pg-work-anim" />}
      </span>
      <span className="pg-current-action__label">{label}</span>
      <StatusIndicator tone={tone} label="" hideLabel size="sm" pulse={working && tone === "active"} />
    </div>
  );
}

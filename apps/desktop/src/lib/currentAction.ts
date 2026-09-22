import type { StatusTone } from "@pegoles/ui";
import type { PresenceState } from "@pegoles/ui";
import type { ActionRequestWire, AgentEvent, StatusPayload } from "../lib/tauri";
import { describeAction, describeEvent } from "../lib/events";

export interface CurrentActionState {
  readonly label: string;
  readonly tone: StatusTone;
  readonly presence: PresenceState;
  readonly working: boolean;
}

/** Latest live (started, not yet terminal) action among task events. */
function liveAction(events: AgentEvent[]): ActionRequestWire | null {
  const terminal = new Set<string>();
  for (const e of events) {
    if (e.type === "action_completed" || e.type === "action_failed" || e.type === "action_denied") {
      const id =
        (e as { action_id?: string }).action_id ??
        (e as { request?: ActionRequestWire }).request?.action_id;
      if (id) terminal.add(id);
    }
  }
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (e.type === "action_started") {
      const s = e as { action_id: string; request: ActionRequestWire };
      if (!terminal.has(s.action_id)) return s.request;
    }
  }
  return null;
}

function actionLabel(action: ActionRequestWire["action"]): CurrentActionState {
  const verb = action.type;
  const typing = verb === "type_text" || verb === "type";
  const waiting = verb === "wait";
  const looking = verb === "observe_screen" || verb === "screenshot" || verb === "get_display_info";
  const label = looking
    ? "Pegoles is looking at the screen"
    : typing
      ? "Pegoles is typing"
      : verb === "scroll"
        ? "Pegoles is scrolling"
        : verb === "drag"
          ? "Pegoles is dragging"
          : verb.startsWith("key")
            ? "Pegoles is using the keyboard"
            : verb === "click" || verb === "double_click"
              ? "Pegoles is clicking"
              : `Pegoles is ${describeAction(action).charAt(0).toLowerCase()}${describeAction(action).slice(1)}`;
  return {
    label,
    tone: "active",
    presence: typing || !waiting ? "acting" : "thinking",
    working: true,
  };
}

/**
 * Derive the compact current-action label from REAL Core state only.
 * A live structured action (started, not terminal) wins over the
 * viewport fallback; unknown states fall back to honest, quiet copy.
 */
export function currentActionFor(
  status: StatusPayload | null,
  connected: boolean,
  taskStatus: string | null,
  latestEvent: AgentEvent | null,
  taskEvents?: AgentEvent[],
): CurrentActionState | null {
  if (!connected || !status) return null;
  if (status.control_owner === "user") {
    return { label: "You're controlling Pegoles Computer", tone: "user", presence: "waitingForUser", working: false };
  }
  if (taskStatus === "waiting_for_approval") {
    return { label: "Pegoles needs your approval", tone: "waiting", presence: "waitingForUser", working: false };
  }
  if (status.agent_busy && taskEvents) {
    const live = liveAction(taskEvents);
    if (live) return actionLabel(live.action);
  }
  switch (status.viewport_state) {
    case "agent_active":
      return {
        label: latestEvent ? `Pegoles is working · ${describeEvent(latestEvent)}` : "Pegoles is working",
        tone: "active",
        presence: "acting",
        working: true,
      };
    case "preparing":
      return { label: "Pegoles is preparing the computer", tone: "active", presence: "thinking", working: true };
    case "starting":
      return { label: "Pegoles is starting the computer", tone: "active", presence: "thinking", working: true };
    case "guest_connecting":
      return { label: "Pegoles is connecting to the computer", tone: "active", presence: "thinking", working: true };
    case "display_starting":
      return { label: "Pegoles is starting the display", tone: "active", presence: "thinking", working: true };
    case "ready":
      return taskStatus === "running"
        ? { label: "Pegoles is thinking", tone: "active", presence: "thinking", working: true }
        : null;
    case "paused":
      return { label: "Computer paused", tone: "paused", presence: "idle", working: false };
    case "error":
      return { label: "Pegoles Computer needs attention", tone: "danger", presence: "error", working: false };
    case "off":
    case "user_controlled":
      return null;
    default:
      return null;
  }
}

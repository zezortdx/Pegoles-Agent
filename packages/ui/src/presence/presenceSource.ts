/**
 * Mapping from REAL app state to presence events.
 *
 * Production rule: presence is decided by the app from facts it already
 * has (Core task status, Core-derived ViewportState, connection to Core,
 * command input focus). These helpers are pure; `usePresence` wires them
 * to the machine. Nothing here invents progress or activity.
 */
import type { PresenceEvent, PresenceState } from "./presenceMachine.js";

/** `pegoles_protocol::TaskStatus` wire values (serde snake_case). */
export type TaskStatusWire =
  | "pending"
  | "running"
  | "waiting_for_approval"
  | "completed"
  | "failed"
  | "cancelled";

/** `pegoles_protocol::ViewportState` wire values (serde snake_case). */
export type ViewportStateWire =
  | "off"
  | "preparing"
  | "starting"
  | "guest_connecting"
  | "display_starting"
  | "ready"
  | "paused"
  | "agent_active"
  | "user_controlled"
  | "error";

export interface PresenceSnapshot {
  /** False when Pegoles Core is unreachable → Offline. */
  readonly online: boolean;
  /** Status of the task the user is looking at (or the most recent one); null = none. */
  readonly task: TaskStatusWire | null;
  /** Core-derived viewport state of the task's computer; null = none. */
  readonly viewport: ViewportStateWire | null;
  /** The command input has focus (the user is about to speak to Pegoles). */
  readonly inputFocused: boolean;
}

export const EMPTY_PRESENCE_SNAPSHOT: PresenceSnapshot = {
  online: true,
  task: null,
  viewport: null,
  inputFocused: false,
};

function isWorking(task: TaskStatusWire | null): boolean {
  return task === "pending" || task === "running" || task === "waiting_for_approval";
}

/**
 * Steady-state presence for a snapshot (no timed Success: completion is a
 * transition, see `presenceEventsForChange`).
 */
export function derivePresence(s: PresenceSnapshot): PresenceState {
  if (!s.online) return "offline";
  if (s.task === "failed" || (isWorking(s.task) && s.viewport === "error")) return "error";
  if (s.task === "waiting_for_approval") return "waitingForUser";
  // A queued task is not evidence of model activity.
  if (s.task === "running") {
    if (s.viewport === "agent_active") return "acting";
    // The human took control of the computer mid-task: the agent waits.
    if (s.viewport === "user_controlled") return "waitingForUser";
    return "thinking";
  }
  return s.inputFocused ? "listening" : "idle";
}

function eventFor(target: PresenceState): PresenceEvent {
  switch (target) {
    case "offline":
      return { type: "offline" };
    case "error":
      return { type: "failed" };
    case "waitingForUser":
      return { type: "needsUser" };
    case "acting":
      return { type: "acting" };
    case "thinking":
      return { type: "thinking" };
    case "listening":
      return { type: "inputFocused" };
    case "success":
      return { type: "succeeded" };
    case "idle":
      return { type: "idleTimeout" };
  }
}

/**
 * Events that move the machine from `prev` to `next`. Completion of a
 * working task emits `succeeded` (Success, then the machine's own timer
 * returns to Idle); cancellation emits `cancelled`.
 */
export function presenceEventsForChange(prev: PresenceSnapshot | null, next: PresenceSnapshot): PresenceEvent[] {
  const events: PresenceEvent[] = [];
  if (!next.online) {
    return prev && !prev.online ? events : [{ type: "offline" }];
  }
  if (prev && !prev.online) events.push({ type: "online" });

  const wasWorking = prev ? isWorking(prev.task) : false;
  if (wasWorking && next.task === "completed") {
    events.push({ type: "succeeded" });
    return events;
  }
  if (wasWorking && next.task === "cancelled") {
    events.push({ type: "cancelled" });
  }
  if (prev?.task !== "running" && next.task === "running") events.push({ type: "taskStarted" });

  const before = prev && prev.online ? derivePresence(prev) : null;
  const target = derivePresence(next);
  if (target !== before || events.length > 0) {
    if (target === "idle" && before === "listening") events.push({ type: "inputBlurred" });
    else events.push(eventFor(target));
  }
  return events;
}

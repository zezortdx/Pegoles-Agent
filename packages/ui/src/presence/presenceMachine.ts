/**
 * Pegoles presence — pure state machine.
 *
 * The mark is a living presence, not a mascot: its state is a readable
 * summary of what the agent is doing. In production the machine is fed
 * ONLY from real app state (task status, computer state, input focus) via
 * `presenceEventsForChange` / `usePresence`; simulated sequences live in
 * the design lab only.
 */

export const PRESENCE_STATES = [
  "idle",
  "listening",
  "thinking",
  "acting",
  "waitingForUser",
  "success",
  "error",
  "offline",
] as const;

export type PresenceState = (typeof PRESENCE_STATES)[number];

export type PresenceEvent =
  | { readonly type: "taskStarted" }
  | { readonly type: "thinking" }
  | { readonly type: "acting" }
  | { readonly type: "needsUser" }
  | { readonly type: "succeeded" }
  | { readonly type: "failed" }
  | { readonly type: "cancelled" }
  | { readonly type: "offline" }
  | { readonly type: "online" }
  | { readonly type: "inputFocused" }
  | { readonly type: "inputBlurred" }
  | { readonly type: "idleTimeout" }
  /** Internal: fired by the timer after a timed state (Success). */
  | { readonly type: "elapsed" };

export type PresenceEventType = PresenceEvent["type"];

/** Success is a brief acknowledgement, then the mark settles back to Idle. */
export const SUCCESS_HOLD_MS = 1200;

type Table = Readonly<Partial<Record<PresenceEventType, Readonly<Partial<Record<PresenceState, PresenceState>>>>>>;

const WORKING: readonly PresenceState[] = ["thinking", "acting", "waitingForUser"];
const ONLINE: readonly PresenceState[] = PRESENCE_STATES.filter((s) => s !== "offline");

function from(states: readonly PresenceState[], to: PresenceState): Partial<Record<PresenceState, PresenceState>> {
  const out: Partial<Record<PresenceState, PresenceState>> = {};
  for (const s of states) out[s] = to;
  return out;
}

/**
 * Valid transitions: TABLE[event][from] = to. Anything not listed is
 * ignored (the state is returned unchanged). Offline swallows every event
 * except `online`.
 */
const TABLE: Table = {
  taskStarted: from(["idle", "listening", "success", "error", "waitingForUser"], "thinking"),
  thinking: from(["idle", "listening", "acting", "waitingForUser", "success", "error"], "thinking"),
  acting: from(["idle", "listening", "thinking", "waitingForUser"], "acting"),
  needsUser: from(["idle", "listening", "thinking", "acting", "error"], "waitingForUser"),
  succeeded: from(WORKING, "success"),
  failed: from(ONLINE.filter((s) => s !== "error"), "error"),
  cancelled: from(WORKING, "idle"),
  offline: from(ONLINE, "offline"),
  online: { offline: "idle" },
  inputFocused: from(["idle", "success", "error"], "listening"),
  inputBlurred: { listening: "idle" },
  idleTimeout: { listening: "idle", error: "idle" },
  elapsed: { success: "idle" },
};

/** Pure transition. Invalid events return `state` unchanged (same value). */
export function transitionPresence(state: PresenceState, event: PresenceEvent): PresenceState {
  return TABLE[event.type]?.[state] ?? state;
}

/** Timed transition owned by a state, if any. */
export function timedTransition(state: PresenceState): { readonly afterMs: number; readonly event: PresenceEvent } | null {
  return state === "success" ? { afterMs: SUCCESS_HOLD_MS, event: { type: "elapsed" } } : null;
}

export function isPresenceState(value: unknown): value is PresenceState {
  return typeof value === "string" && (PRESENCE_STATES as readonly string[]).includes(value);
}

/** Short human label, used in the mark's accessible name. */
export const PRESENCE_LABEL: Readonly<Record<PresenceState, string>> = {
  idle: "Idle",
  listening: "Listening",
  thinking: "Thinking",
  acting: "Working",
  waitingForUser: "Waiting for you",
  success: "Done",
  error: "Needs attention",
  offline: "Offline",
};

// ---------------------------------------------------------------------------
// Runtime (timers) — tiny, framework-free, injectable clock.
// ---------------------------------------------------------------------------

export interface PresenceTimers {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const defaultTimers: PresenceTimers = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface PresenceMachine {
  readonly state: PresenceState;
  /** Returns the resulting state. Invalid events are ignored silently. */
  send(event: PresenceEvent): PresenceState;
  subscribe(listener: (state: PresenceState) => void): () => void;
  /**
   * Releases the pending timer and all listeners. Not terminal: a later
   * `send` re-arms timers (keeps React StrictMode remounts safe).
   */
  dispose(): void;
}

export function createPresenceMachine(
  initial: PresenceState = "idle",
  timers: PresenceTimers = defaultTimers,
): PresenceMachine {
  let state = initial;
  let timer: unknown = null;
  const listeners = new Set<(s: PresenceState) => void>();

  const arm = () => {
    if (timer !== null) {
      timers.clearTimeout(timer);
      timer = null;
    }
    const timed = timedTransition(state);
    if (timed) {
      timer = timers.setTimeout(() => {
        timer = null;
        machine.send(timed.event);
      }, timed.afterMs);
    }
  };

  const machine: PresenceMachine = {
    get state() {
      return state;
    },
    send(event) {
      const next = transitionPresence(state, event);
      if (next === state) return state;
      state = next;
      arm();
      for (const listener of listeners) listener(state);
      return state;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    dispose() {
      if (timer !== null) timers.clearTimeout(timer);
      timer = null;
      listeners.clear();
    },
  };
  arm();
  return machine;
}

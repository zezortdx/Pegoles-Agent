/**
 * AgentCursor state machine (pure). The AgentCursor represents THE AGENT
 * acting inside its own computer — never the human's pointer.
 */

export const CURSOR_STATES = ["hidden", "idle", "moving", "clicking", "dragging", "typing", "waiting"] as const;
export type CursorState = (typeof CURSOR_STATES)[number];

export type CursorEvent =
  | "show"
  | "hide"
  | "move"
  | "arrive"
  | "click"
  | "clickEnd"
  | "dragStart"
  | "dragEnd"
  | "typeStart"
  | "typeEnd"
  | "wait";

type Table = Readonly<Record<CursorState, Readonly<Partial<Record<CursorEvent, CursorState>>>>>;

/**
 * Valid transitions. Anything not listed is ignored. `hide` works from
 * every visible state (a human taking control hides the agent cursor at
 * once); only `show` leaves `hidden`.
 */
const TABLE: Table = {
  hidden: { show: "idle" },
  idle: { hide: "hidden", move: "moving", click: "clicking", dragStart: "dragging", typeStart: "typing", wait: "waiting" },
  moving: { hide: "hidden", move: "moving", arrive: "idle", click: "clicking", dragStart: "dragging", wait: "waiting" },
  clicking: { hide: "hidden", clickEnd: "idle", move: "moving", dragStart: "dragging", click: "clicking" },
  dragging: { hide: "hidden", dragEnd: "idle", dragStart: "dragging" },
  typing: { hide: "hidden", typeEnd: "idle", move: "moving", click: "clicking", wait: "waiting" },
  waiting: { hide: "hidden", move: "moving", click: "clicking", dragStart: "dragging", typeStart: "typing" },
};

export function transitionCursor(state: CursorState, event: CursorEvent): CursorState {
  return TABLE[state][event] ?? state;
}

export function canTransition(state: CursorState, event: CursorEvent): boolean {
  return TABLE[state][event] !== undefined;
}

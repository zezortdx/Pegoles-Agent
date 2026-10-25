/**
 * Real agent actions → the preview's agent cursor.
 *
 * Maps Core's `pegoles://event` stream to AgentCursor actions, one computer
 * at a time. Only `action_started` moves the cursor: that event is emitted
 * after policy allowed the action and Core began executing it, so the
 * cursor shows what really happens and nothing that was denied. The real
 * input never waits for the cursor; this only listens.
 *
 * Coordinates stay normalized (as in the protocol); the overlay maps them
 * into the drawn frame, so the guest resolution, its DPI and the host's
 * scale never matter here.
 */
import { listen } from "@tauri-apps/api/event";
import type { AgentCursorAction, AgentCursorSource } from "@pegoles/ui";
import type { ActionRequestWire, AgentEvent } from "./tauri";

type Payload = Record<string, unknown>;

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function point(x: unknown, y: unknown): { x: number; y: number } | null {
  const px = num(x);
  const py = num(y);
  return px === null || py === null ? null : { x: px, y: py };
}

const TYPING = new Set(["type_text", "key_press", "key_chord"]);
const LOOKING = new Set(["observe_screen", "get_display_info"]);
const ENDED = new Set(["failed", "cancelled"]);

/** Cursor actions for one real action that just started. */
function actionsFor(action: ActionRequestWire["action"]): AgentCursorAction[] {
  const a = action as Payload;
  switch (action.type) {
    case "move_pointer": {
      const at = point(a.x, a.y);
      return at ? [{ kind: "move", at }] : [];
    }
    case "click":
    case "double_click": {
      const at = point(a.x, a.y);
      return at ? [{ kind: "click", at, count: action.type === "double_click" ? 2 : 1 }] : [];
    }
    case "mouse_down": {
      const at = point(a.x, a.y);
      return at ? [{ kind: "press", at }] : [];
    }
    case "mouse_up": {
      const at = point(a.x, a.y);
      return at ? [{ kind: "release", at }] : [];
    }
    case "drag": {
      const from = point(a.from_x, a.from_y);
      const to = point(a.to_x, a.to_y);
      return from && to ? [{ kind: "drag", from, to, durationMs: num(a.duration_ms) ?? 0 }] : [];
    }
    case "scroll": {
      const at = point(a.x, a.y);
      return at ? [{ kind: "scroll", at, dx: num(a.delta_x) ?? 0, dy: num(a.delta_y) ?? 0 }] : [];
    }
    case "wait":
      return [{ kind: "think" }];
    default:
      if (TYPING.has(action.type)) return [{ kind: "typing", active: true }];
      if (LOOKING.has(action.type)) return [{ kind: "observe" }];
      return [];
  }
}

/**
 * Pure mapping of Core events to cursor actions for ONE computer. Events of
 * other computers are ignored; the task whose actions drive the cursor
 * decides when it settles (done) or stops (failed, cancelled).
 */
export class CursorEventMapper {
  private task: string | null = null;
  private readonly typing = new Set<string>();

  constructor(private readonly computerId: string) {}

  map(event: AgentEvent): AgentCursorAction[] {
    const e = event as Payload & { type: string };
    switch (e.type) {
      case "action_started": {
        const request = e.request as ActionRequestWire | undefined;
        if (!request || request.computer_id !== this.computerId) return [];
        this.task = request.task_id;
        const actions = actionsFor(request.action);
        if (TYPING.has(request.action.type)) this.typing.add(request.action_id);
        return actions;
      }
      case "action_completed":
      case "action_failed":
      case "action_denied": {
        const request = e.request as ActionRequestWire | undefined;
        if (!request || request.computer_id !== this.computerId) return [];
        const id = (typeof e.action_id === "string" ? e.action_id : null) ?? request.action_id;
        if (this.typing.delete(id)) return [{ kind: "typing", active: false }];
        // Between actions the planner is deciding the next step: quieter.
        return e.type === "action_denied" ? [] : [{ kind: "think" }];
      }
      case "task_status_changed": {
        if (e.task_id !== this.task) return [];
        if (e.to === "completed") return this.end({ kind: "done" });
        if (typeof e.to === "string" && ENDED.has(e.to)) return this.end({ kind: "stop" });
        if (e.to === "waiting_for_approval") return [{ kind: "think" }];
        return [];
      }
      case "computer_state_changed":
        return e.computer_id === this.computerId && e.to !== "running" ? this.end({ kind: "stop" }) : [];
      case "control_ownership_changed":
        return e.computer_id === this.computerId && e.to === "user" ? this.end({ kind: "stop" }) : [];
      case "guest_runtime_disconnected":
        return e.computer_id === this.computerId ? this.end({ kind: "stop" }) : [];
      default:
        return [];
    }
  }

  private end(action: AgentCursorAction): AgentCursorAction[] {
    this.typing.clear();
    this.task = null;
    return [action];
  }
}

// One Tauri listener for the whole app, fanned out to every preview.
const subscribers = new Set<(event: AgentEvent) => void>();
let listening = false;

function ensureListening(): void {
  if (listening) return;
  listening = true;
  listen<AgentEvent>("pegoles://event", (e) => {
    for (const subscriber of [...subscribers]) {
      try {
        subscriber(e.payload);
      } catch {
        // One broken preview must not starve the others.
      }
    }
  }).catch(() => {
    listening = false;
  });
}

/**
 * The cursor feed for one computer. Each subscription starts clean (no
 * history is replayed), so a preview mounted mid-task picks up at the next
 * real action, and a replaced computer never inherits the old one's cursor.
 */
export function agentCursorSource(computerId: string): AgentCursorSource {
  return {
    id: `agent-actions:${computerId}`,
    subscribe(listener) {
      const mapper = new CursorEventMapper(computerId);
      const onEvent = (event: AgentEvent) => {
        for (const action of mapper.map(event)) listener(action);
      };
      subscribers.add(onEvent);
      ensureListening();
      return () => {
        subscribers.delete(onEvent);
      };
    },
  };
}

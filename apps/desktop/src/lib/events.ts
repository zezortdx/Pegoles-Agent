import type { ActionRequestWire, AgentEvent } from "../lib/tauri";

/** Friendly one-liner for a structured action (mirrors the Rust
 * `ComputerAction::describe`; never raw protocol JSON). Secret typing
 * shows no text (the wire already arrives redacted). */
export function describeAction(action: { type: string; [k: string]: unknown }): string {
  switch (action.type) {
    case "screenshot":
    case "observe_screen":
      return "Looking at the screen";
    case "get_display_info":
      return "Checking display size";
    case "move_pointer":
      return "Moving pointer";
    case "click":
      return "Clicking";
    case "double_click":
      return "Double-clicking";
    case "mouse_down":
      return "Pressing pointer";
    case "mouse_up":
      return "Releasing pointer";
    case "drag":
      return "Dragging";
    case "scroll":
      return "Scrolling";
    case "key_press":
      return typeof action.key === "string" ? `Pressing ${action.key}` : "Pressing key";
    case "key_chord":
      return Array.isArray(action.keys) ? `Pressing ${(action.keys as string[]).join("+")}` : "Pressing keys";
    case "type_text":
    case "type": {
      const text = typeof action.text === "string" ? action.text : "";
      if (text.length === 0) return "Typing";
      const short = [...text].slice(0, 42).join("");
      return text.length > 42 ? `Typing “${short}…”` : `Typing “${short}”`;
    }
    case "wait":
      return "Waiting";
    case "open_url":
      return "Opening link";
    case "shell":
      return "Running shell command";
    case "read_file":
      return "Reading file";
    case "write_file":
      return "Writing file";
    default:
      return String(action.type).replaceAll("_", " ");
  }
}

function requestOf(e: AgentEvent): ActionRequestWire | null {
  if ("request" in e && typeof e.request === "object" && e.request !== null) {
    return e.request as ActionRequestWire;
  }
  return null;
}

export function describeEvent(e: AgentEvent): string {
  switch (e.type) {
    case "computer_created":
      return "Computer created";
    case "computer_state_changed": {
      const s = e as Extract<AgentEvent, { type: "computer_state_changed" }>;
      if (s.from === "stopped" && s.to === "stopped") return "Computer ready";
      return `Computer ${s.from} → ${s.to}`;
    }
    case "task_created":
      return `Task created: ${(e as { title: string }).title}`;
    case "task_status_changed": {
      const t = e as { from: string; to: string };
      return `Task ${t.from} → ${t.to}`;
    }
    case "action_requested":
    case "action_evaluated":
    case "action_started": {
      const req = requestOf(e);
      return req ? describeAction(req.action) : "Action requested";
    }
    case "action_completed": {
      const c = e as Extract<AgentEvent, { type: "action_completed" }>;
      const verb = describeAction(c.request.action);
      if (c.result.outcome === "executed") return verb;
      if (c.result.outcome === "needs_approval") return `${verb} — needs approval`;
      return `${verb} — ${c.result.outcome}`;
    }
    case "action_denied":
      return `${describeAction((e as Extract<AgentEvent, { type: "action_denied" }>).request.action)} — denied`;
    case "action_failed": {
      const f = e as Extract<AgentEvent, { type: "action_failed" }>;
      const verb = f.request ? describeAction(f.request.action) : "Action";
      return f.error.startsWith("interrupted") ? `${verb} — interrupted` : `${verb} — failed`;
    }
    case "approval_requested":
      return "Approval requested";
    case "frame_observed": {
      const f = e as Extract<AgentEvent, { type: "frame_observed" }>;
      return `Screen captured (${f.frame.width_px}×${f.frame.height_px})`;
    }
    case "input_capability_changed": {
      const c = e as Extract<AgentEvent, { type: "input_capability_changed" }>;
      return c.available ? "Agent input ready" : "Agent input unavailable";
    }
    case "guest_runtime_waiting":
      return "Waiting for guest runtime";
    case "guest_runtime_connected":
      return "Guest runtime connected";
    case "guest_handshake_completed":
      return "Handshake completed";
    case "guest_runtime_ready": {
      const r = e as { ready_in_ms: number };
      return typeof r.ready_in_ms === "number"
        ? `Debian ready (${(r.ready_in_ms / 1000).toFixed(1)}s)`
        : "Debian ready";
    }
    case "guest_runtime_disconnected": {
      const d = e as { reason?: string };
      return d.reason ? `Guest runtime disconnected (${d.reason})` : "Guest runtime disconnected";
    }
    case "guest_runtime_incompatible": {
      const i = e as { guest_version?: number };
      return `Guest runtime incompatible (protocol ${i.guest_version ?? "?"})`;
    }
    case "guest_runtime_error": {
      const m = e as { message?: string };
      return m.message ? `Guest runtime error: ${m.message}` : "Guest runtime error";
    }
    default:
      return e.type.replaceAll("_", " ");
  }
}

export function eventTime(e: AgentEvent): string {
  const at = (e as { at?: string }).at;
  if (!at) return "";
  const d = new Date(at);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

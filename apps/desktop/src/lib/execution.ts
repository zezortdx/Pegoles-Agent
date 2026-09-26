import type { ActionRequestWire, AgentEvent, AgentMessageKind } from "./tauri";

export type Capability = "Computer" | "Web" | "Files" | "Shell";

/** Validate the open-ended event envelope at the UI boundary. */
export function requestOf(event: AgentEvent): ActionRequestWire | null {
  const request = "request" in event ? event.request : null;
  if (!request || typeof request !== "object") return null;
  const r = request as Partial<ActionRequestWire>;
  return typeof r.task_id === "string" && typeof r.action_id === "string" && r.action && typeof r.action.type === "string"
    ? r as ActionRequestWire : null;
}

/** Model narration for one task. `text` is untrusted: render it as plain text only. */
export interface AgentMessage {
  readonly taskId: string;
  readonly kind: AgentMessageKind;
  readonly text: string;
  readonly at: string;
}

const MESSAGE_KINDS: ReadonlySet<string> = new Set<AgentMessageKind>(["progress", "summary", "error"]);

/** An `agent_message` validated at the boundary; null for anything else or an empty note. */
export function agentMessageOf(event: AgentEvent): AgentMessage | null {
  if (event.type !== "agent_message") return null;
  const { task_id: taskId, kind, text, at } = event as Record<string, unknown>;
  if (typeof taskId !== "string" || typeof text !== "string" || typeof at !== "string") return null;
  if (typeof kind !== "string" || !MESSAGE_KINDS.has(kind)) return null;
  const trimmed = text.trim();
  return trimmed ? { taskId, kind: kind as AgentMessageKind, text: trimmed, at } : null;
}

/** The first line of a note, short enough for one line of UI. */
export function firstLine(text: string, limit = 160): string {
  const line = text.split(/\r?\n/).map((part) => part.trim()).find(Boolean) ?? "";
  const chars = [...line];
  return chars.length > limit ? `${chars.slice(0, limit - 1).join("").trimEnd()}…` : line;
}

export function eventsForTask(events: AgentEvent[], taskId: string): AgentEvent[] {
  const ids = new Set(events.flatMap((event) => {
    const req = requestOf(event);
    return req?.task_id === taskId ? [req.action_id] : [];
  }));
  return events.filter((event) => ("task_id" in event && event.task_id === taskId) || requestOf(event)?.task_id === taskId ||
    (event.type === "frame_observed" && typeof event.action_id === "string" && ids.has(event.action_id)));
}

export function capabilityFor(verb: string): Capability | undefined {
  if (["read_file", "write_file", "list_directory"].includes(verb)) return "Files";
  if (["shell", "run_command"].includes(verb)) return "Shell";
  if (["open_url", "browser_navigate", "web_search"].includes(verb)) return "Web";
  if (["screenshot", "observe_screen", "get_display_info", "move_pointer", "click", "double_click", "mouse_down", "mouse_up", "drag", "scroll", "key_press", "key_chord", "type_text", "type"].includes(verb)) return "Computer";
  return undefined;
}

export interface FileArtifact { id: string; path: string; message: string }
export function fileArtifacts(events: AgentEvent[]): FileArtifact[] {
  const files = new Map<string, FileArtifact>();
  for (const event of events) {
    const req = requestOf(event);
    const result = ("result" in event ? event.result : undefined) as { success?: boolean; outcome?: string; message?: string } | undefined;
    if (event.type !== "action_completed" || req?.action.type !== "write_file" || !result?.success || result.outcome !== "executed") continue;
    const path = req.action.path;
    if (typeof path === "string") files.set(path, { id: req.action_id, path, message: result.message ?? "" });
  }
  return [...files.values()];
}

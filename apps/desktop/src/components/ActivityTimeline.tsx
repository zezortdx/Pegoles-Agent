import { ActivityItem, ActivityList } from "@pegoles/ui";
import { describeAction, describeEvent } from "../lib/events";
import type { ActionRequestWire, AgentEvent } from "../lib/tauri";

interface Row {
  key: string;
  kind: "task" | "agent" | "computer" | "guest" | "display" | "control" | "image" | "error" | "info";
  title: string;
  at: string;
  current: boolean;
}

function timeLabel(at: string): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Collapse each action's lifecycle (requested→…→terminal) into ONE row
 * showing its latest state; drop bare pointer moves (the cursor shows
 * movement — the timeline must not spam movement rows). */
function summarize(events: AgentEvent[]): Row[] {
  const ordered = [...events].reverse().slice(0, 200);
  const actionState = new Map<string, { req: ActionRequestWire; terminal: string | null; at: string }>();
  const order: string[] = [];
  const passthrough: Row[] = [];
  for (const e of ordered) {
    const t = e.type;
    if (
      t === "action_requested" ||
      t === "action_evaluated" ||
      t === "action_started" ||
      t === "action_completed" ||
      t === "action_denied" ||
      t === "action_failed"
    ) {
      const id =
        (e as { action_id?: string }).action_id ??
        (e as { request?: ActionRequestWire }).request?.action_id;
      const req =
        (e as { request?: ActionRequestWire }).request ?? actionState.get(id ?? "")?.req;
      if (!id || !req) continue;
      // Bare moves never earn a row.
      if (req.action.type === "move_pointer") continue;
      if (!actionState.has(id)) order.push(id);
      const prev = actionState.get(id);
      let terminal: string | null = prev?.terminal ?? null;
      let at = prev?.at ?? "";
      if (t === "action_completed") {
        const result = (e as unknown as { result: { outcome: string; completed_at: string } }).result;
        terminal = result.outcome === "executed" ? null : result.outcome;
        at = result.completed_at ?? at;
      } else if (t === "action_denied") {
        terminal = "denied";
      } else if (t === "action_failed") {
        const err = (e as { error: string }).error;
        terminal = err.startsWith("interrupted") ? "interrupted" : "failed";
        at = (e as { at?: string }).at ?? at;
      } else if (t === "action_started") {
        at = (e as { at?: string }).at ?? at;
      }
      actionState.set(id, { req, terminal, at });
      continue;
    }
    const at = typeof (e as { at?: unknown }).at === "string" ? (e as { at: string }).at : "";
    passthrough.push({
      key: `${String((e as { at?: unknown }).at)}-${passthrough.length}`,
      kind: e.type.includes("error") ? "error" : e.type.startsWith("task") ? "task" : "computer",
      title: describeEvent(e),
      at,
      current: false,
    });
  }
  const actionRows: Row[] = order.map((id, i) => {
    const s = actionState.get(id);
    const failed = s?.terminal === "failed" || s?.terminal === "denied";
    const suffix = s?.terminal && s.terminal !== "executed" ? ` — ${s.terminal}` : "";
    return {
      key: `action-${id}`,
      kind: failed ? "error" : "agent",
      title: s ? describeAction(s.req.action) + suffix : "Action",
      at: s?.at ?? "",
      current: i === 0,
    };
  });
  return [...actionRows, ...passthrough].slice(0, 100);
}

export function ActivityTimeline({ events, liveCount = 1 }: { events: AgentEvent[]; liveCount?: number }) {
  if (!events.length) return <p className="empty-copy">No activity yet. Computer and task updates will appear here.</p>;
  const rows = summarize(events);
  if (!rows.length)
    return <p className="empty-copy">No activity yet. Computer and task updates will appear here.</p>;
  return (
    <ActivityList label="Recent activity">
      {rows.map((row, i) => (
        <ActivityItem
          key={row.key}
          kind={row.kind}
          title={row.title}
          at={row.at}
          timeLabel={timeLabel(row.at)}
          current={i < liveCount && row.current}
        />
      ))}
    </ActivityList>
  );
}

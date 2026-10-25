/**
 * Development-only presence lab (#/dev/experience). Never loaded by the
 * production entry.
 * Query options (for headless captures):
 *   ?mode=thinking&reduced=1&grid=0&size=48&ribbon=play&boop=<ms>
 * (size sets the hero size; ribbon=play starts the live ribbon sequence;
 * boop presses the hero once after <ms>, to capture it mid-squash).
 */
import { useEffect, useMemo, useState } from "react";
import { FluxGlassRoot } from "@pegoles/ui";
import { PegolesPresence, PRESENCE_MODES, PresenceMark, type PresenceMode } from "../presence";
import { ThoughtRibbon } from "../thought/ThoughtRibbon";
import { ribbonModel } from "../state/ribbon";
import type { TaskStatusWire } from "@pegoles/ui";
import type { AgentEvent, AgentTask, StatusPayload } from "../lib/tauri";
import "../styles/tokens.css";

/** Unique string that must never appear in a production bundle. */
export const EXPERIENCE_LAB_MARKER = "__PEGOLES_EXPERIENCE_LAB__";

const params = new URLSearchParams(window.location.hash.split("?")[1] ?? "");
const initialMode = (PRESENCE_MODES as readonly string[]).includes(params.get("mode") ?? "") ? params.get("mode") as PresenceMode : "idle";
const heroSize = Number(params.get("size")) > 0 ? Number(params.get("size")) : 152;

const page: React.CSSProperties = { minHeight: "100vh", background: "var(--bg-0)", color: "var(--text-1)", font: "var(--type-body)", padding: "32px 40px 64px" };
const label: React.CSSProperties = { font: "var(--type-small)", color: "var(--text-3)" };
const control: React.CSSProperties = { font: "var(--type-small)", color: "var(--text-1)", background: "var(--bg-2)", border: "var(--border-strong)", borderRadius: "var(--radius-control)", padding: "6px 10px" };

// ── Ribbon fixtures: real event shapes through the real model ─────────
const AT = "2026-09-24T10:00:00Z";
const TASK: AgentTask = { id: "lab", title: "Tidy Downloads", status: "running", created_at: AT, updated_at: AT };
const BUSY = { agent_busy: true, control_owner: "agent", model: "configured" } as StatusPayload;
const QUIET = { agent_busy: false, control_owner: "none", model: "configured" } as StatusPayload;
const request = (id: string, type: string, extra: Record<string, unknown> = {}) =>
  ({ action_id: id, task_id: "lab", computer_id: "vm", action: { type, ...extra }, requested_at: AT });
const ev = {
  req: (id: string, type: string, extra?: Record<string, unknown>): AgentEvent => ({ type: "action_requested", request: request(id, type, extra) }),
  start: (id: string, type: string): AgentEvent => ({ type: "action_started", action_id: id, request: request(id, type), at: AT }),
  done: (id: string, type: string): AgentEvent => ({ type: "action_completed", request: request(id, type), result: { action_id: id, success: true, outcome: "executed", message: "", duration_ms: 9 } }),
  approval: (id: string, type: string): AgentEvent => ({ type: "action_evaluated", request: request(id, type), verdict: { decision: "require_approval", risk: "medium", reason: "Writes a file" } }),
  fail: (id: string, type: string): AgentEvent => ({ type: "action_failed", action_id: id, request: request(id, type), error: "not found", at: AT }),
  deny: (id: string, type: string): AgentEvent => ({ type: "action_denied", request: request(id, type), reason: "host path" }),
};
const HISTORY = [ev.req("a1", "list_directory"), ev.start("a1", "list_directory"), ev.done("a1", "list_directory"),
  ev.req("a2", "read_file"), ev.start("a2", "read_file"), ev.done("a2", "read_file")];

interface Scenario { readonly name: string; readonly events: AgentEvent[]; readonly status?: TaskStatusWire; readonly core?: StatusPayload; readonly connected?: boolean }
const SCENARIOS: readonly Scenario[] = [
  { name: "thinking · nothing in flight", events: [] },
  { name: "sprouting", events: [ev.req("a1", "open_url")], core: QUIET },
  { name: "running (Files)", events: [ev.req("a1", "read_file"), ev.start("a1", "read_file")] },
  { name: "running after history", events: [...HISTORY, ev.req("a3", "screenshot"), ev.start("a3", "screenshot")] },
  { name: "approval", events: [...HISTORY, ev.req("a3", "write_file"), ev.approval("a3", "write_file")], status: "waiting_for_approval", core: QUIET },
  { name: "completed · back to thinking", events: HISTORY, core: QUIET },
  { name: "failed", events: [...HISTORY, ev.req("a3", "open_url"), ev.start("a3", "open_url"), ev.fail("a3", "open_url")], core: QUIET },
  { name: "blocked", events: [ev.req("a1", "shell"), ev.deny("a1", "shell")], core: QUIET },
  { name: "done", events: [...HISTORY, ev.req("a3", "write_file"), ev.done("a3", "write_file")], status: "completed", core: QUIET },
  { name: "offline", events: HISTORY, connected: false },
];

/** A real-looking sequence, one event every 1.1 s, looping. */
const SEQUENCE: readonly { events: AgentEvent[]; core: StatusPayload; status?: TaskStatusWire }[] = (() => {
  const steps: AgentEvent[][] = [[], [ev.req("s1", "list_directory")], [ev.start("s1", "list_directory")], [ev.done("s1", "list_directory")],
    [ev.req("s2", "read_file")], [ev.start("s2", "read_file")], [ev.done("s2", "read_file")],
    [ev.req("s3", "write_file"), ev.approval("s3", "write_file")], [ev.start("s3", "write_file")], [ev.done("s3", "write_file")]];
  let acc: AgentEvent[] = [];
  return steps.map((step, i) => {
    acc = [...acc, ...step];
    const last = step[step.length - 1];
    return { events: acc, core: last?.type === "action_started" ? BUSY : QUIET, status: i === steps.length - 1 ? "completed" : i === 7 ? "waiting_for_approval" : undefined };
  });
})();

function LiveRibbon({ reduced }: { reduced: boolean }) {
  const [step, setStep] = useState(0);
  const [playing, setPlaying] = useState(params.get("ribbon") === "play");
  useEffect(() => {
    if (!playing) return;
    const timer = window.setInterval(() => setStep((s) => (s + 1) % SEQUENCE.length), 1100);
    return () => window.clearInterval(timer);
  }, [playing]);
  const frame = SEQUENCE[step];
  const model = useMemo(() => ribbonModel({ task: { ...TASK, status: frame.status ?? "running" }, events: frame.events, status: frame.core, connected: true }), [frame]);
  return <div data-lab-ribbon-live style={{ display: "flex", gap: 24, alignItems: "center" }}>
    <ThoughtRibbon model={model} reducedMotion={reduced} />
    <button type="button" style={control} onClick={() => setPlaying((p) => !p)}>{playing ? "Pause" : "Play"}</button>
    <button type="button" style={control} onClick={() => setStep((s) => (s + 1) % SEQUENCE.length)}>Step</button>
    <span style={label}>step {step + 1}/{SEQUENCE.length} · {frame.events[frame.events.length - 1]?.type ?? "task running"}</span>
  </div>;
}

export function ExperienceLab() {
  const [mode, setMode] = useState<PresenceMode>(initialMode);
  const [reduced, setReduced] = useState(params.get("reduced") === "1");
  const [pulse, setPulse] = useState(0);
  const [look, setLook] = useState(false);
  const showGrid = params.get("grid") !== "0";
  const [nudge, setNudge] = useState(0);
  const [typing, setTyping] = useState(false);
  const [presses, setPresses] = useState(0);
  useEffect(() => {
    const at = Number(params.get("boop"));
    if (!(at > 0)) return;
    const timer = window.setTimeout(() => document.querySelector<HTMLElement>("[data-lab-hero] .presence")?.click(), at);
    return () => window.clearTimeout(timer);
  }, []);

  return <FluxGlassRoot tier="full" reducedMotion={reduced}>
    <div style={page} data-lab-marker={EXPERIENCE_LAB_MARKER}>
      <header style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
        <strong style={{ font: "var(--type-heading)" }}>Presence lab</strong>
        <span style={label}>Development only, no backend</span>
        <select aria-label="Mode" style={control} value={mode} onChange={(e) => setMode(e.target.value as PresenceMode)}>
          {PRESENCE_MODES.map((m) => <option key={m}>{m}</option>)}
        </select>
        <label style={{ font: "var(--type-small)" }}><input type="checkbox" checked={reduced} onChange={(e) => setReduced(e.target.checked)} /> Reduced motion</label>
        <label style={{ font: "var(--type-small)" }}><input type="checkbox" checked={look} onChange={(e) => setLook(e.target.checked)} /> Look at composer</label>
        <button type="button" style={control} onClick={() => setPulse((p) => p + 1)}>Pulse</button>
      </header>

      <section data-lab-hero style={{ display: "grid", placeItems: "center", alignContent: "center", gap: 40, height: 460 }}>
        <PegolesPresence mode={mode} size={heroSize} interactive pressable onPress={() => setPresses((p) => p + 1)}
          pulse={pulse} nudge={nudge} look={look || typing ? { x: 0, y: 1 } : null} />
        <input data-lab-input aria-label="Type to nudge" placeholder="Type here: every keystroke nudges Pegoles" style={{ ...control, width: 420, padding: "10px 14px" }}
          onFocus={() => setTyping(true)} onBlur={() => setTyping(false)} onChange={() => setNudge((n) => n + 1)} />
        <span style={label}>presses {presses} · nudges {nudge}</span>
      </section>

      <p style={label}>Sizes · PresenceMark at 16–36, PegolesPresence at 48+</p>
      <section data-lab-sizes style={{ display: "flex", alignItems: "center", gap: 36, margin: "20px 0 40px", padding: "20px 24px", width: "fit-content" }}>
        {[16, 18, 24, 32, 36].map((s) => <PresenceMark key={s} mode={mode} size={s} />)}
        {[48, 64].map((s) => <PegolesPresence key={s} mode={mode} size={s} field={false} />)}
      </section>

      <p style={label}>Marks · 18 px in every mode</p>
      <section data-lab-marks style={{ display: "flex", flexWrap: "wrap", gap: 18, margin: "16px 0 40px", padding: "12px 16px", width: "fit-content" }}>
        {PRESENCE_MODES.map((m) => <span key={m} title={m} style={{ display: "grid", justifyItems: "center", gap: 6 }}>
          <PresenceMark mode={m} size={18} /><span style={{ ...label, fontSize: 10 }}>{m}</span>
        </span>)}
      </section>

      <p style={label}>Thought Ribbon · every phase, from real event shapes</p>
      <section data-lab-ribbons style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))", gap: "28px 24px", margin: "20px 0 28px", padding: "8px 12px" }}>
        {SCENARIOS.map((s) => <figure key={s.name} data-lab-ribbon={s.name} style={{ display: "grid", gap: 8, margin: 0 }}>
          <ThoughtRibbon reducedMotion={reduced} model={ribbonModel({ task: { ...TASK, status: s.status ?? "running" }, events: s.events, status: s.core ?? BUSY, connected: s.connected ?? true })} />
          <figcaption style={label}>{s.name}</figcaption>
        </figure>)}
      </section>
      <section style={{ margin: "0 0 56px", padding: "8px 12px" }}><LiveRibbon reduced={reduced} /></section>

      {showGrid && <>
        <p style={label}>Every mode</p>
        <section data-lab-modes style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(190px, 1fr))", gap: "40px 16px", margin: "24px 0 56px" }}>
          {PRESENCE_MODES.map((m) => <figure key={m} style={{ display: "grid", justifyItems: "center", gap: 28, margin: 0 }}>
            <PegolesPresence mode={m} size={112} pulse={pulse} />
            <figcaption style={label}>{m}</figcaption>
          </figure>)}
        </section>
      </>}
    </div>
  </FluxGlassRoot>;
}

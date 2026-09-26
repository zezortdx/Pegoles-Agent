/**
 * Design lab · Eyes & Hands (Phase 5). DEV ONLY.
 *
 * Every button drives the REAL backend (execute_action / capture_screen /
 * run_input_script against Pegoles Core). Nothing here simulates: on a
 * headless mock or an old guest the buttons report honest structured
 * errors. Diagnostics show live guest/viewport/cursor/ownership facts.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { ActionResultWire, FrameMetaWire } from "../../lib/tauri";
import { api, debugApi } from "../../lib/tauri";
import { LabSection, SimulatedTag } from "../lab/controls";

interface Diag {
  available: boolean;
  frame_available: boolean;
  agent_busy: boolean;
  pressed_clean: boolean;
  audit_len: number;
  last_frame: FrameMetaWire | null;
}

const button: React.CSSProperties = {
  padding: "7px 12px",
  borderRadius: 10,
  border: "1px solid rgb(244 247 252 / 0.12)",
  background: "rgb(244 247 252 / 0.05)",
  color: "var(--pg-text-primary)",
  font: "inherit",
  fontSize: 13,
};

const surface: React.CSSProperties = {
  borderRadius: 14,
  background: "rgb(var(--pg-surface-default-rgb, 7 11 20) / 0.72)",
  boxShadow: "inset 0 0 0 1px var(--pg-border-subtle, rgb(244 247 252 / 0.08))",
  padding: 14,
};

function norm(x: number, y: number) {
  return { type: "move_pointer", x, y };
}

export function EyesHandsSection() {
  const [diag, setDiag] = useState<Diag | null>(null);
  const [lastAction, setLastAction] = useState<ActionResultWire | null>(null);
  const [frame, setFrame] = useState<{ meta: FrameMetaWire; png_base64: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const d = await debugApi.inputStatus();
      if (mounted.current) setDiag(d);
    } catch (e) {
      if (mounted.current) setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = useCallback(
    async (fn: () => Promise<ActionResultWire>) => {
      setBusy(true);
      setError(null);
      try {
        const result = await fn();
        if (mounted.current) setLastAction(result);
      } catch (e) {
        if (mounted.current) setError(String(e));
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [refresh],
  );

  const capture = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const shot = await api.captureScreen();
      if (mounted.current) setFrame(shot);
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
      void refresh();
    }
  }, [refresh]);

  const runDemo = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const steps = await debugApi.demoScriptSteps();
      const report = await debugApi.runInputScript(steps);
      if (mounted.current) {
        setLastAction(report.results[report.results.length - 1] ?? null);
        setError(
          report.aborted_at !== null && report.aborted_at !== undefined
            ? `Demo aborted at step ${report.aborted_at} (${report.steps_executed}/${report.steps_total} executed)`
            : `Demo complete (${report.steps_executed}/${report.steps_total})`,
        );
      }
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
      void refresh();
    }
  }, [refresh]);

  return (
    <LabSection
      id="lab-eyes-hands"
      index="14"
      title="Eyes & Hands"
      lead="Real computer observation and input against the isolated Pegoles Computer. No fixtures here: every control calls Core and reports what the guest actually did."
    >
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap", marginBottom: 14 }}>
        <SimulatedTag>Real backend calls · dev only</SimulatedTag>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 300px", gap: 16, alignItems: "start" }}>
        <div style={{ display: "grid", gap: 12 }}>
          <div style={{ ...surface, display: "flex", gap: 8, flexWrap: "wrap" }}>
            <button type="button" style={button} disabled={busy} onClick={() => void capture()}>
              Capture Screen
            </button>
            <button type="button" style={button} disabled={busy} onClick={() => void run(() => debugApi.executeAction(norm(0.5, 0.5)))}>
              Move center
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() => void run(() => debugApi.executeAction({ type: "click", x: 0.5, y: 0.5, button: "primary" }))}
            >
              Click center
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() => void run(() => debugApi.executeAction({ type: "double_click", x: 0.5, y: 0.5, button: "primary" }))}
            >
              Double-click
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() => void run(() => debugApi.executeAction({ type: "type_text", text: "echo hello", sensitive: false }))}
            >
              Type “echo hello”
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() => void run(() => debugApi.executeAction({ type: "key_press", key: "Enter" }))}
            >
              Press Enter
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() => void run(() => debugApi.executeAction({ type: "scroll", x: 0.5, y: 0.5, delta_x: 0, delta_y: 3 }))}
            >
              Scroll
            </button>
            <button
              type="button"
              style={button}
              disabled={busy}
              onClick={() =>
                void run(() =>
                  debugApi.executeAction({
                    type: "drag",
                    from_x: 0.3,
                    from_y: 0.5,
                    to_x: 0.7,
                    to_y: 0.5,
                    button: "primary",
                    duration_ms: 600,
                  }),
                )
              }
            >
              Drag
            </button>
            <button type="button" style={{ ...button, background: "rgb(1 95 248 / 0.35)" }} disabled={busy} onClick={() => void runDemo()}>
              Run Demo
            </button>
            <button type="button" style={button} disabled={busy} onClick={() => void api.cancelAgentInput().then(() => refresh())}>
              Cancel
            </button>
          </div>
          {frame && (
            <figure style={{ ...surface, margin: 0 }}>
              <img
                src={`data:image/png;base64,${frame.png_base64}`}
                alt={`Guest frame ${frame.meta.width_px}x${frame.meta.height_px}`}
                style={{ width: "100%", borderRadius: 8, display: "block" }}
              />
              <figcaption className="lab-mono" style={{ marginTop: 8 }}>
                {frame.meta.frame_id} · {frame.meta.width_px}×{frame.meta.height_px} · {frame.meta.byte_len} bytes ·{" "}
                {frame.meta.capture_latency_ms} ms
              </figcaption>
            </figure>
          )}
          {lastAction && (
            <dl style={{ ...surface, margin: 0, display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 12px" }} className="lab-mono">
              <dt>action</dt>
              <dd style={{ margin: 0 }}>{lastAction.action_id}</dd>
              <dt>outcome</dt>
              <dd style={{ margin: 0 }}>{lastAction.outcome}</dd>
              <dt>duration</dt>
              <dd style={{ margin: 0 }}>{lastAction.duration_ms} ms</dd>
              <dt>message</dt>
              <dd style={{ margin: 0 }}>{lastAction.message || lastAction.error}</dd>
            </dl>
          )}
          {error && (
            <p role="alert" style={{ ...surface, color: "var(--pg-text-primary)", fontSize: 13 }}>
              {error}
            </p>
          )}
        </div>
        <dl style={{ ...surface, margin: 0, display: "grid", gridTemplateColumns: "auto 1fr", gap: "8px 12px" }} className="lab-mono">
          <dt>input</dt>
          <dd style={{ margin: 0 }}>{diag ? (diag.available ? "available" : "unavailable") : "…"}</dd>
          <dt>capture</dt>
          <dd style={{ margin: 0 }}>{diag ? (diag.frame_available ? "available" : "unavailable") : "…"}</dd>
          <dt>agent</dt>
          <dd style={{ margin: 0 }}>{diag ? (diag.agent_busy ? "busy" : "idle") : "…"}</dd>
          <dt>pressed</dt>
          <dd style={{ margin: 0 }}>{diag ? (diag.pressed_clean ? "clean" : "HELD") : "…"}</dd>
          <dt>audit rows</dt>
          <dd style={{ margin: 0 }}>{diag?.audit_len ?? "…"}</dd>
          <dt>last frame</dt>
          <dd style={{ margin: 0, overflowWrap: "anywhere" }}>{diag?.last_frame?.frame_id ?? "—"}</dd>
          <dt>frame size</dt>
          <dd style={{ margin: 0 }}>
            {diag?.last_frame ? `${diag.last_frame.width_px}×${diag.last_frame.height_px}` : "—"}
          </dd>
        </dl>
      </div>
      <p className="lab-footnote">
        Coordinates are normalized agent space (0–1). Cursor motion in the production UI comes from the same action
        events this panel triggers — watch the workspace while running the demo.
      </p>
    </LabSection>
  );
}

/**
 * Design lab · AgentCursor (stream D2). DEV ONLY.
 *
 * The cursor here is driven by a SIMULATED fixture script. In the app the
 * cursor mounts only from real agent-action events (AgentCursorOverlay +
 * productionAgentCursorSource), and Phase 4 has none.
 */
import { Profiler, useEffect, useRef, useState, type CSSProperties, type MutableRefObject } from "react";
import {
  AgentCursorLayer,
  EFFECTS_TIERS,
  effectsTiers,
  productionAgentCursorSource,
  trailLengthFor,
  useFluxGlass,
  type AgentCursorHandle,
  type EffectsTier,
} from "@pegoles/ui";
import { Segmented, SimulatedTag, Switch } from "../lab/controls";

const surface: CSSProperties = {
  borderRadius: 22,
  background: "rgb(var(--pg-surface-default-rgb, 7 11 20) / 0.72)",
  boxShadow: "inset 0 0 0 1px var(--pg-border-subtle, rgb(244 247 252 / 0.08))",
};

const button: CSSProperties = {
  padding: "7px 12px",
  borderRadius: 10,
  border: "1px solid rgb(244 247 252 / 0.12)",
  background: "rgb(244 247 252 / 0.05)",
  color: "var(--pg-text-primary)",
  font: "inherit",
  fontSize: 13,
};

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

/** A still, simulated guest desktop the fixture cursor acts on. */
function FakeDesktop({ targets }: { readonly targets: MutableRefObject<Record<string, HTMLElement | null>> }) {
  const set = (id: string) => (node: HTMLElement | null) => {
    targets.current[id] = node;
  };
  const line: CSSProperties = { height: 8, borderRadius: 4, background: "rgb(244 247 252 / 0.08)" };
  return (
    <div aria-hidden="true" style={{ position: "absolute", inset: 0, padding: 22, display: "grid", gridTemplateRows: "auto 1fr", gap: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {[0, 1, 2].map((i) => (
          <span key={i} style={{ width: 10, height: 10, borderRadius: 5, background: "rgb(244 247 252 / 0.14)" }} />
        ))}
        <span style={{ marginLeft: 12, fontSize: 12, color: "var(--pg-text-secondary)" }}>guest · foot — simulated</span>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1.2fr 1fr", gap: 18 }}>
        <div style={{ display: "grid", alignContent: "start", gap: 12 }}>
          <div ref={set("field")} style={{ height: 36, borderRadius: 10, boxShadow: "inset 0 0 0 1px rgb(244 247 252 / 0.14)" }} />
          <div style={{ ...line, width: "82%" }} />
          <div style={{ ...line, width: "64%" }} />
          <div style={{ ...line, width: "74%" }} />
          <div
            ref={set("button")}
            style={{ justifySelf: "start", marginTop: 8, padding: "9px 18px", borderRadius: 10, background: "rgb(1 95 248 / 0.28)", fontSize: 12 }}
          >
            Save
          </div>
        </div>
        <div style={{ display: "grid", alignContent: "start", gap: 10 }}>
          {["notes.txt", "report.md", "archive"].map((name, i) => (
            <div
              key={name}
              ref={set(`file${i}`)}
              style={{ padding: "10px 12px", borderRadius: 10, background: "rgb(244 247 252 / 0.04)", fontSize: 12, color: "var(--pg-text-secondary)" }}
            >
              {name}
            </div>
          ))}
          <div ref={set("folder")} style={{ marginTop: 8, height: 64, borderRadius: 12, border: "1px dashed rgb(244 247 252 / 0.16)" }} />
        </div>
      </div>
    </div>
  );
}

export function AgentCursorSection() {
  const ctx = useFluxGlass();
  const [tier, setTier] = useState<EffectsTier>(ctx.tier);
  const [reducedMotion, setReducedMotion] = useState(ctx.reducedMotion);
  const stageRef = useRef<HTMLDivElement>(null);
  const cursorRef = useRef<AgentCursorHandle>(null);
  const targets = useRef<Record<string, HTMLElement | null>>({});
  const renders = useRef(0);
  const readout = useRef<{ renders: HTMLElement | null; frames: HTMLElement | null; loop: HTMLElement | null; state: HTMLElement | null }>({
    renders: null,
    frames: null,
    loop: null,
    state: null,
  });
  const running = useRef(0);
  const bind = (key: keyof typeof readout.current) => (node: HTMLElement | null) => {
    readout.current[key] = node;
  };

  useEffect(() => setTier(ctx.tier), [ctx.tier]);
  useEffect(() => setReducedMotion(ctx.reducedMotion), [ctx.reducedMotion]);

  // Perf readout written straight to the DOM (a React readout would re-render the lab).
  useEffect(() => {
    const cursor = cursorRef.current;
    const write = () => {
      const r = readout.current;
      if (r.renders) r.renders.textContent = String(renders.current);
      if (r.frames) r.frames.textContent = String(cursor?.frameCount ?? 0);
      if (r.loop) r.loop.textContent = cursor?.isAnimating ? "running" : "asleep";
      if (r.state) r.state.textContent = cursor?.state ?? "hidden";
    };
    write();
    const unsubscribe = cursor?.subscribe(write);
    const poll = window.setInterval(write, 250);
    return () => {
      unsubscribe?.();
      window.clearInterval(poll);
    };
  }, []);

  /** Centre of a fixture target in overlay coordinates. */
  const at = (id: string) => {
    const stage = stageRef.current?.getBoundingClientRect();
    const el = targets.current[id]?.getBoundingClientRect();
    if (!stage || !el) return { x: 40, y: 40 };
    return { x: Math.round(el.left - stage.left + el.width * 0.6), y: Math.round(el.top - stage.top + el.height * 0.55) };
  };

  const perform = async (fn: (c: AgentCursorHandle) => unknown) => {
    const cursor = cursorRef.current;
    if (!cursor) return;
    if (cursor.state === "hidden") cursor.show();
    await fn(cursor);
  };

  const runScript = async () => {
    const token = ++running.current;
    const cursor = cursorRef.current;
    if (!cursor) return;
    const alive = () => running.current === token;
    cursor.show();
    const steps: (() => unknown)[] = [
      () => cursor.moveTo(at("field").x, at("field").y),
      () => cursor.click(),
      () => sleep(220),
      () => cursor.typing(true),
      () => sleep(1300),
      () => cursor.typing(false),
      () => cursor.moveTo(at("button").x, at("button").y),
      () => cursor.click(),
      () => sleep(200),
      () => cursor.click(),
      () => sleep(420),
      () => cursor.moveTo(at("file0").x, at("file0").y),
      () => sleep(160),
      // Scroll: glide down the file list before dragging.
      () => cursor.moveTo(at("file1").x, at("file1").y),
      () => sleep(160),
      () => cursor.moveTo(at("file2").x, at("file2").y),
      () => sleep(160),
      () => cursor.dragTo(at("folder").x, at("folder").y),
      () => sleep(600),
      () => cursor.wait(),
    ];
    for (const step of steps) {
      if (!alive()) return;
      await step();
    }
  };

  const trail = trailLengthFor(tier, reducedMotion);

  return (
    <>
      <header className="lab-section__header" style={{ gridTemplateColumns: "minmax(0, 1fr)" }}>
        <h2 className="lab-section__title">Agent cursor</h2>
        <p className="lab-section__lead">
          The AgentCursor is the agent, never the human: a luminous white-to-cyan core with an electric edge, moving on a
          critically damped spring. Positions bypass React entirely — an imperative controller writes transforms to an
          isolated overlay and its frame loop sleeps the moment the cursor settles. In production it appears only from real
          agent actions; Phase 4 has none, so the app never shows it.
        </p>
      </header>

      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 14, marginBottom: 20 }}>
        <Segmented
          label="Cursor tier"
          value={tier}
          onChange={setTier}
          size="sm"
          options={EFFECTS_TIERS.map((t) => ({ value: t, label: effectsTiers[t].label }))}
        />
        <div style={{ width: 210 }}>
          <Switch label="Reduced motion" hint="Moves become fades" checked={reducedMotion} onChange={setReducedMotion} />
        </div>
        <SimulatedTag>Simulated fixture · lab only</SimulatedTag>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 240px", gap: 20, alignItems: "start" }}>
        <div ref={stageRef} style={{ ...surface, position: "relative", height: 400, overflow: "hidden", background: "#02040A" }}>
          <FakeDesktop targets={targets} />
          <Profiler
            id="agent-cursor"
            onRender={() => {
              renders.current += 1;
            }}
          >
            <AgentCursorLayer ref={cursorRef} effectsTier={tier} reducedMotion={reducedMotion} />
          </Profiler>
        </div>

        <div style={{ display: "grid", gap: 16 }}>
          <div style={{ ...surface, padding: 16, display: "grid", gap: 8 }}>
            <button type="button" style={{ ...button, background: "rgb(1 95 248 / 0.35)" }} onClick={() => void runScript()}>
              Run fixture script
            </button>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              <button type="button" style={button} onClick={() => void perform((c) => c.moveTo(at("button").x, at("button").y))}>
                Move
              </button>
              <button type="button" style={button} onClick={() => void perform((c) => c.click())}>
                Click
              </button>
              <button
                type="button"
                style={button}
                onClick={() => void perform(async (c) => { c.click(); await sleep(140); c.click(); })}
              >
                Double-click
              </button>
              <button
                type="button"
                style={button}
                onClick={() => void perform(async (c) => { await c.moveTo(at("file0").x, at("file0").y); await c.moveTo(at("file2").x, at("file2").y); })}
              >
                Scroll
              </button>
              <button type="button" style={button} onClick={() => void perform((c) => c.dragTo(at("folder").x, at("folder").y))}>
                Drag
              </button>
              <button
                type="button"
                style={button}
                onClick={() => void perform((c) => c.typing(c.state !== "typing"))}
              >
                Typing
              </button>
              <button type="button" style={button} onClick={() => void perform((c) => c.wait())}>
                Wait
              </button>
              <button
                type="button"
                style={button}
                onClick={() => {
                  running.current += 1;
                  const c = cursorRef.current;
                  if (!c) return;
                  if (c.state === "hidden") c.show();
                  else c.hide();
                }}
              >
                Hide / show
              </button>
            </div>
          </div>

          <dl style={{ ...surface, margin: 0, padding: 16, display: "grid", gridTemplateColumns: "1fr auto", rowGap: 8, columnGap: 12 }}>
            <dt className="lab-mono">React renders</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("renders")}>
              0
            </dd>
            <dt className="lab-mono">Loop frames</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("frames")}>
              0
            </dd>
            <dt className="lab-mono">Frame loop</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("loop")}>
              asleep
            </dd>
            <dt className="lab-mono">State</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("state")}>
              hidden
            </dd>
            <dt className="lab-mono">Trail samples</dt>
            <dd className="lab-mono" style={{ margin: 0 }}>
              {trail}
            </dd>
            <dt className="lab-mono">Production source</dt>
            <dd className="lab-mono" style={{ margin: 0 }}>
              {productionAgentCursorSource.id} · 0 events
            </dd>
          </dl>
        </div>
      </div>
      <p className="lab-footnote">
        React renders stay constant while the cursor moves (they change only when you switch tier or reduced motion here).
        Loop frames grow only while it travels; &ldquo;asleep&rdquo; means no requestAnimationFrame is scheduled. Clicks are a 170 ms
        refraction ring, drags leave a short direction path that fades, waiting shows a still attention ring.
      </p>
    </>
  );
}

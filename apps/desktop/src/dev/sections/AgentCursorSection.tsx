/**
 * Design lab · AgentCursor. DEV ONLY.
 *
 * The cursor here is driven by a SIMULATED fixture. In the app it moves
 * only from real agent actions (Core's `action_started` events through
 * apps/desktop/src/lib/agentCursorFeed.ts).
 */
import { Profiler, useEffect, useRef, useState, type CSSProperties, type MutableRefObject } from "react";
import {
  AgentCursorLayer,
  EFFECTS_TIERS,
  GLIDE,
  PULSE_MS,
  effectsTiers,
  useFluxGlass,
  type AgentCursorHandle,
  type EffectsTier,
  type Point,
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
  const readout = useRef<{ renders: HTMLElement | null; frames: HTMLElement | null; loop: HTMLElement | null; state: HTMLElement | null; backlog: HTMLElement | null }>({
    renders: null,
    frames: null,
    loop: null,
    state: null,
    backlog: null,
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
      if (r.backlog) r.backlog.textContent = String(cursor?.backlog ?? 0);
    };
    write();
    const unsubscribe = cursor?.subscribe(write);
    const poll = window.setInterval(write, 120);
    return () => {
      unsubscribe?.();
      window.clearInterval(poll);
    };
  }, []);

  /** A fixture target as a normalized point of the simulated guest frame. */
  const at = (id: string): Point => {
    const stage = stageRef.current?.getBoundingClientRect();
    const el = targets.current[id]?.getBoundingClientRect();
    if (!stage || !el || stage.width === 0 || stage.height === 0) return { x: 0.1, y: 0.1 };
    return { x: (el.left - stage.left + el.width * 0.6) / stage.width, y: (el.top - stage.top + el.height * 0.55) / stage.height };
  };

  const act = (fn: (c: AgentCursorHandle) => void) => {
    running.current += 1;
    const cursor = cursorRef.current;
    if (cursor) fn(cursor);
  };

  const runScript = async () => {
    const token = ++running.current;
    const cursor = cursorRef.current;
    if (!cursor) return;
    const alive = () => running.current === token;
    const steps: (() => unknown)[] = [
      () => cursor.observe(),
      () => sleep(300),
      () => cursor.click(at("field")),
      () => sleep(260),
      () => cursor.typing(true),
      () => sleep(1100),
      () => cursor.typing(false),
      () => sleep(500),
      () => cursor.click(at("button"), 2),
      () => sleep(600),
      () => cursor.scroll(at("file1"), 0, 3),
      () => sleep(500),
      () => cursor.drag(at("file2"), at("folder"), 700),
      () => sleep(1100),
      () => cursor.think(),
      () => sleep(900),
      () => cursor.done(),
    ];
    for (const step of steps) {
      if (!alive()) return;
      await step();
    }
  };

  /** Actions arriving faster than any glide: the cursor must follow the newest, never queue. */
  const burst = () =>
    act((c) => {
      const ids = ["field", "file0", "button", "file2", "folder", "file1"];
      ids.forEach((id, i) => window.setTimeout(() => c.moveTo(at(id)), i * 40));
      window.setTimeout(() => c.click(at("button")), ids.length * 40);
    });

  return (
    <>
      <header className="lab-section__header" style={{ gridTemplateColumns: "minmax(0, 1fr)" }}>
        <h2 className="lab-section__title">Agent cursor</h2>
        <p className="lab-section__lead">
          The agent&rsquo;s pointer over its computer&rsquo;s preview: a small silver arrow with the mark&rsquo;s soft light and a dark
          hairline, so it reads on light and dark screens. It glides from where it is to each real action&rsquo;s coordinate
          ({GLIDE.baseMs}&ndash;{GLIDE.maxMs} ms, longer only for longer moves), retargets instead of queueing, pulses a click for
          {" "}{PULSE_MS} ms at the exact point and shows a drag as one continuous stroke. Positions bypass React: an imperative
          controller writes transforms and its frame loop sleeps when the cursor lands.
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
          <Switch label="Reduced motion" hint="Moves land at once" checked={reducedMotion} onChange={setReducedMotion} />
        </div>
        <SimulatedTag>Simulated fixture · lab only</SimulatedTag>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 240px", gap: 20, alignItems: "start" }}>
        <div ref={stageRef} data-lab-cursor-stage style={{ ...surface, position: "relative", height: 400, overflow: "hidden", background: "#02040A" }}>
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
            <button type="button" style={{ ...button, background: "rgb(255 255 255 / 0.12)" }} onClick={() => void runScript()}>
              Run fixture script
            </button>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              <button type="button" style={button} onClick={() => act((c) => c.moveTo(at("button")))}>Move</button>
              <button type="button" style={button} onClick={() => act((c) => c.click(at("field")))}>Click</button>
              <button type="button" style={button} onClick={() => act((c) => c.click(at("button"), 2))}>Double-click</button>
              <button type="button" style={button} onClick={() => act((c) => c.scroll(at("file1"), 0, 3))}>Scroll</button>
              <button type="button" style={button} onClick={() => act((c) => c.drag(at("file0"), at("folder"), 700))}>Drag</button>
              <button type="button" style={button} onClick={() => act((c) => c.typing(c.state !== "typing"))}>Typing</button>
              <button type="button" style={button} onClick={() => act((c) => c.think())}>Think</button>
              <button type="button" style={button} onClick={burst}>Burst</button>
              <button type="button" style={button} onClick={() => act((c) => c.done())}>Done</button>
              <button type="button" style={button} onClick={() => act((c) => c.stop())}>Stop</button>
            </div>
          </div>

          <dl style={{ ...surface, margin: 0, padding: 16, display: "grid", gridTemplateColumns: "1fr auto", rowGap: 8, columnGap: 12 }}>
            <dt className="lab-mono">React renders</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("renders")}>0</dd>
            <dt className="lab-mono">Loop frames</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("frames")}>0</dd>
            <dt className="lab-mono">Frame loop</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("loop")}>asleep</dd>
            <dt className="lab-mono">State</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("state")}>hidden</dd>
            <dt className="lab-mono">Backlog</dt>
            <dd className="lab-mono" style={{ margin: 0, color: "var(--pg-text-primary)" }} ref={bind("backlog")}>0</dd>
          </dl>
        </div>
      </div>
      <p className="lab-footnote">
        React renders stay constant while the cursor moves (they change only when you switch tier or reduced motion here).
        Loop frames grow only while it travels; &ldquo;asleep&rdquo; means no requestAnimationFrame is scheduled. Backlog never
        exceeds one effect: a new action replaces the old glide.
      </p>
    </>
  );
}

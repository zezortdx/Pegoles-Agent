/**
 * Design lab · Presence (stream D2). DEV ONLY — simulated states live here
 * and nowhere else; the app derives presence from real state (usePresence).
 */
import { useEffect, useRef, useState, type CSSProperties } from "react";
import {
  EFFECTS_TIERS,
  PRESENCE_LABEL,
  PRESENCE_STATES,
  PegolesMark,
  effectsTiers,
  lookToward,
  presenceVisual,
  useFluxGlass,
  type EffectsTier,
  type PresenceState,
  type Vec2,
} from "@pegoles/ui";
import { Segmented, SimulatedTag, Switch } from "../lab/controls";

const HERO = 160;
const RAMP = [20, 32, 64, 160] as const;
const MATRIX_SIZE = 56;

/** Simulated lifecycle for the lab only. */
const FIXTURE: readonly { readonly state: PresenceState; readonly ms: number }[] = [
  { state: "idle", ms: 2400 },
  { state: "listening", ms: 2200 },
  { state: "thinking", ms: 3200 },
  { state: "acting", ms: 3000 },
  { state: "waitingForUser", ms: 2600 },
  { state: "acting", ms: 2000 },
  { state: "success", ms: 1200 },
  { state: "idle", ms: 1800 },
  { state: "error", ms: 2600 },
  { state: "offline", ms: 2400 },
];

const ATTENTION = [
  { value: "90", label: "Below" },
  { value: "0", label: "Right" },
  { value: "180", label: "Left" },
  { value: "270", label: "Above" },
] as const;

type AttentionChoice = (typeof ATTENTION)[number]["value"];

const TARGETS = [
  { id: "left", label: "Plan", style: { left: 0, top: "42%" } },
  { id: "right", label: "Computer", style: { right: 0, top: "30%" } },
  { id: "below", label: "Command", style: { left: "50%", bottom: 0, transform: "translateX(-50%)" } },
] as const;

const surface: CSSProperties = {
  borderRadius: 22,
  background: "rgb(var(--pg-surface-default-rgb, 7 11 20) / 0.72)",
  boxShadow: "inset 0 0 0 1px var(--pg-border-subtle, rgb(244 247 252 / 0.08))",
};

const caption: CSSProperties = {
  fontFamily: "var(--pg-font-mono)",
  fontSize: 11,
  letterSpacing: "0.06em",
  textTransform: "uppercase",
  color: "var(--pg-text-secondary)",
};

function Hero({
  state,
  tier,
  reducedMotion,
  attention,
}: {
  readonly state: PresenceState;
  readonly tier: EffectsTier;
  readonly reducedMotion: boolean;
  readonly attention: number;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const [lookAt, setLookAt] = useState<Vec2 | null>(null);

  const lookAtTarget = (el: HTMLElement) => {
    const box = stage.current?.getBoundingClientRect();
    const t = el.getBoundingClientRect();
    if (!box) return;
    const center = { x: box.left + box.width / 2, y: box.top + box.height / 2 };
    setLookAt(lookToward(center, { x: t.left + t.width / 2, y: t.top + t.height / 2 }, 200));
  };

  return (
    <div ref={stage} style={{ ...surface, position: "relative", height: 380, display: "grid", placeItems: "center" }}>
      <PegolesMark size={HERO} state={state} effectsTier={tier} reducedMotion={reducedMotion} lookAt={lookAt} attention={attention} />
      {TARGETS.map((t) => (
        <button
          key={t.id}
          type="button"
          onPointerEnter={(e) => lookAtTarget(e.currentTarget)}
          onPointerLeave={() => setLookAt(null)}
          onFocus={(e) => lookAtTarget(e.currentTarget)}
          onBlur={() => setLookAt(null)}
          style={{
            position: "absolute",
            margin: 18,
            padding: "10px 14px",
            borderRadius: 12,
            border: "1px solid rgb(244 247 252 / 0.1)",
            background: "rgb(244 247 252 / 0.04)",
            color: "var(--pg-text-secondary)",
            font: "inherit",
            fontSize: 13,
            cursor: "default",
            ...t.style,
          }}
        >
          {t.label}
        </button>
      ))}
      <p style={{ ...caption, position: "absolute", left: 20, top: 18, margin: 0 }}>
        {PRESENCE_LABEL[state]} · {tier}
        {reducedMotion ? " · reduced motion" : ""}
      </p>
      <p style={{ ...caption, position: "absolute", right: 20, top: 18, margin: 0, textTransform: "none", letterSpacing: 0 }}>
        Hover a panel: the eyes drift ≤ 3 px toward it
      </p>
    </div>
  );
}

function Matrix() {
  const columns = EFFECTS_TIERS.flatMap((tier) => [
    { tier, reducedMotion: false },
    { tier, reducedMotion: true },
  ]);
  return (
    <div style={{ ...surface, overflowX: "auto", padding: "18px 20px" }}>
      <table style={{ borderCollapse: "collapse", width: "100%", minWidth: 720 }}>
        <thead>
          <tr>
            <th style={{ ...caption, textAlign: "left", padding: "0 8px 14px 0" }}>State</th>
            {columns.map((c) => (
              <th key={`${c.tier}-${c.reducedMotion}`} style={{ ...caption, padding: "0 6px 14px", fontWeight: 500 }}>
                {effectsTiers[c.tier].label}
                <span style={{ display: "block", opacity: 0.7 }}>{c.reducedMotion ? "reduced motion" : "motion"}</span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {PRESENCE_STATES.map((state) => (
            <tr key={state}>
              <th scope="row" style={{ textAlign: "left", padding: "10px 8px 10px 0", fontWeight: 500, fontSize: 13, whiteSpace: "nowrap" }}>
                {PRESENCE_LABEL[state]}
              </th>
              {columns.map((c) => {
                const visual = presenceVisual({ state, tier: c.tier, reducedMotion: c.reducedMotion, size: MATRIX_SIZE });
                const loops = visual.animations.filter((a) => a.iterations === "infinite").map((a) => a.keyframes.replace("pgm-", ""));
                return (
                  <td key={`${c.tier}-${c.reducedMotion}`} style={{ textAlign: "center", padding: "10px 6px" }}>
                    <PegolesMark size={MATRIX_SIZE} state={state} effectsTier={c.tier} reducedMotion={c.reducedMotion} />
                    <span className="lab-mono" style={{ display: "block", marginTop: 6, fontSize: 10, opacity: 0.8 }}>
                      {loops.length ? loops.join(" · ") : "static"}
                    </span>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SizeRamp({ state, tier, reducedMotion }: { readonly state: PresenceState; readonly tier: EffectsTier; readonly reducedMotion: boolean }) {
  return (
    <div style={{ ...surface, display: "grid", gridTemplateColumns: "180px minmax(0, 1fr)", overflow: "hidden" }}>
      <nav
        aria-label="Nav rail preview"
        style={{ display: "flex", flexDirection: "column", gap: 6, padding: 14, boxShadow: "inset -1px 0 0 rgb(244 247 252 / 0.06)" }}
      >
        {["Pegoles", "Home", "Computer"].map((label, i) => (
          <span
            key={label}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "8px 10px",
              borderRadius: 10,
              fontSize: 13,
              color: i === 0 ? "var(--pg-text-primary)" : "var(--pg-text-secondary)",
              background: i === 0 ? "rgb(244 247 252 / 0.05)" : undefined,
            }}
          >
            {i === 0 ? (
              <PegolesMark size={20} state={state} effectsTier={tier} reducedMotion={reducedMotion} decorative />
            ) : (
              <span style={{ width: 20, height: 20, borderRadius: 6, boxShadow: "inset 0 0 0 1.5px rgb(244 247 252 / 0.2)" }} />
            )}
            {label}
          </span>
        ))}
      </nav>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-around", gap: 32, padding: "40px 32px 28px" }}>
        {RAMP.map((size) => (
          <figure key={size} style={{ margin: 0, display: "grid", justifyItems: "center", gap: 14 }}>
            <PegolesMark size={size} state={state} effectsTier={tier} reducedMotion={reducedMotion} />
            <figcaption className="lab-mono">{size} px</figcaption>
          </figure>
        ))}
      </div>
    </div>
  );
}

export function PresenceSection() {
  const ctx = useFluxGlass();
  const [state, setState] = useState<PresenceState>("idle");
  const [tier, setTier] = useState<EffectsTier>(ctx.tier);
  const [reducedMotion, setReducedMotion] = useState(ctx.reducedMotion);
  const [attention, setAttention] = useState<AttentionChoice>("90");
  const [playing, setPlaying] = useState(false);

  useEffect(() => setTier(ctx.tier), [ctx.tier]);
  useEffect(() => setReducedMotion(ctx.reducedMotion), [ctx.reducedMotion]);

  useEffect(() => {
    if (!playing) return undefined;
    let index = 0;
    let timer = 0;
    const step = () => {
      const entry = FIXTURE[index % FIXTURE.length];
      if (!entry) return;
      setState(entry.state);
      index += 1;
      timer = window.setTimeout(step, entry.ms);
    };
    step();
    return () => window.clearTimeout(timer);
  }, [playing]);

  return (
    <>
      <header className="lab-section__header" style={{ gridTemplateColumns: "minmax(0, 1fr)" }}>
        <h2 className="lab-section__title">Presence</h2>
        <p className="lab-section__lead">
          The mark is Pegoles&rsquo; presence, not a mascot: light, never a face. Idle breathes slowly; Listening leans its
          light toward the command surface; Thinking sends light around the ring; Acting carries more blue energy; Waiting
          settles and focuses; Success is a brief cyan acknowledgement; Error lowers energy and adds a badge — the logo is
          never painted red; Offline is almost unlit. Vector traced from the source PNG (IoU ≥ 0.985).
        </p>
      </header>

      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 14, marginBottom: 16 }}>
        <Segmented
          label="Presence state"
          value={state}
          onChange={(s) => {
            setPlaying(false);
            setState(s);
          }}
          size="sm"
          options={PRESENCE_STATES.map((s) => ({ value: s, label: PRESENCE_LABEL[s] }))}
        />
        <SimulatedTag>Simulated states · lab only</SimulatedTag>
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 14, marginBottom: 20 }}>
        <Segmented
          label="Preview tier"
          value={tier}
          onChange={setTier}
          size="sm"
          options={EFFECTS_TIERS.map((t) => ({ value: t, label: effectsTiers[t].label }))}
        />
        <Segmented label="Attention direction" value={attention} onChange={setAttention} size="sm" options={ATTENTION} />
        <div style={{ width: 190 }}>
          <Switch label="Reduced motion" checked={reducedMotion} onChange={setReducedMotion} />
        </div>
        <div style={{ width: 190 }}>
          <Switch label="Play lifecycle" hint="Fixture sequence" checked={playing} onChange={setPlaying} />
        </div>
      </div>

      <Hero state={state} tier={tier} reducedMotion={reducedMotion} attention={Number(attention)} />

      <h3 className="lab-subhead">Sizes · nav rail to hero</h3>
      <SizeRamp state={state} tier={tier} reducedMotion={reducedMotion} />

      <h3 className="lab-subhead">States × tiers × motion</h3>
      <Matrix />
      <p className="lab-footnote">
        Loops listed under each mark are the only continuous animations for that cell. Every loop is opacity or a transform
        on a pre-rendered layer, runs through the ambient/work gates (paused when the window is idle, hidden or the mark is
        offscreen), and none runs in Minimal or under reduced motion.
      </p>
    </>
  );
}

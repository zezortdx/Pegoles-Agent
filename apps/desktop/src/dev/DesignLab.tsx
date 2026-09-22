/**
 * Pegoles Flux Glass — Design Lab (`#/dev/design`). DEV ONLY.
 *
 * Loaded through a dynamic import guarded by `import.meta.env.DEV` in
 * main.tsx, so production bundles never contain it (verified by
 * prodBundle.test.ts searching dist/ for DESIGN_LAB_MARKER). Simulated
 * state is allowed here and nowhere else.
 */
import { useEffect, useMemo, useState } from "react";
import {
  FluxGlassRoot,
  isEffectsTier,
  PegolesMark,
  selectEffectsTier,
  usePrefersReducedMotion,
  VIEWPORT_STATES,
  type EffectsTier,
  type ViewportState,
} from "@pegoles/ui";
import { GOVERNOR_FIXTURES, type GovernorFixture } from "./fixtures";
import { Segmented, Switch } from "./lab/controls";
import { StatusStrip } from "./lab/StatusStrip";
import { ActivitySection } from "./sections/ActivitySection";
import { AgentCursorSection } from "./sections/AgentCursorSection";
import { ColorSection } from "./sections/ColorSection";
import { CommandSection } from "./sections/CommandSection";
import { ButtonsSection, StatusSection } from "./sections/ControlsSection";
import { EffectsSection } from "./sections/EffectsSection";
import { MaterialsSection } from "./sections/MaterialsSection";
import { MotionSection } from "./sections/MotionSection";
import { PresenceSection } from "./sections/PresenceSection";
import { TypeSection } from "./sections/TypeSection";
import { ViewportSection } from "./sections/ViewportSection";
import { WorkspaceSection } from "./sections/WorkspaceSection";
import { EyesHandsSection } from "./sections/EyesHandsSection";
import "./designLab.css";

/** Unique string that must never appear in a production bundle. */
export const DESIGN_LAB_MARKER = "__PEGOLES_DESIGN_LAB__";

type TierChoice = "auto" | EffectsTier;
type MotionChoice = "system" | "reduce" | "allow";

interface LabParams {
  readonly tier?: TierChoice;
  readonly motion?: MotionChoice;
  readonly governor?: GovernorFixture["id"];
  readonly section?: string;
  readonly state?: ViewportState;
}

/** `#/dev/design?tier=minimal&motion=reduce&governor=lowEnd&section=computer&state=agent_active` */
function readParams(): LabParams {
  const query = window.location.hash.split("?")[1] ?? "";
  const params = new URLSearchParams(query);
  const tier = params.get("tier");
  const motion = params.get("motion");
  const governor = params.get("governor");
  const state = params.get("state");
  return {
    tier: tier === "auto" || isEffectsTier(tier) ? tier : undefined,
    motion: motion === "system" || motion === "reduce" || motion === "allow" ? motion : undefined,
    governor: GOVERNOR_FIXTURES.find((g) => g.id === governor)?.id,
    section: params.get("section") ?? undefined,
    state: VIEWPORT_STATES.find((s) => s === state),
  };
}

const NAV = [
  { id: "color", label: "Color" },
  { id: "type", label: "Typography" },
  { id: "materials", label: "Materials" },
  { id: "buttons", label: "Buttons" },
  { id: "status", label: "Status" },
  { id: "command", label: "Command → Task" },
  { id: "activity", label: "Activity" },
  { id: "computer", label: "Pegoles Computer" },
  { id: "motion", label: "Motion" },
  { id: "effects", label: "Effects tiers" },
  { id: "presence", label: "Presence" },
  { id: "cursor", label: "Agent cursor" },
  { id: "workspace", label: "Task Workspace" },
  { id: "eyes-hands", label: "Eyes & Hands" },
] as const;

function scrollToSection(id: string): void {
  document.getElementById(`lab-${id}`)?.scrollIntoView({ block: "start" });
}

export function DesignLab() {
  const initial = useMemo(readParams, []);
  const [tierChoice, setTierChoice] = useState<TierChoice>(initial.tier ?? "auto");
  const [governorId, setGovernorId] = useState<GovernorFixture["id"]>(initial.governor ?? "high");
  const [lowPower, setLowPower] = useState(false);
  const [motionChoice, setMotionChoice] = useState<MotionChoice>(initial.motion ?? "system");
  const systemReducedMotion = usePrefersReducedMotion();
  const reducedMotion = motionChoice === "system" ? systemReducedMotion : motionChoice === "reduce";
  const governor = GOVERNOR_FIXTURES.find((g) => g.id === governorId) ?? GOVERNOR_FIXTURES[0];

  const selection = selectEffectsTier({
    recommended: governor?.recommended ?? null,
    userOverride: tierChoice === "auto" ? null : tierChoice,
    prefersReducedMotion: reducedMotion,
    lowPower,
  });

  useEffect(() => {
    document.title = "Flux Glass · Design Lab";
    if (initial.section) scrollToSection(initial.section);
  }, [initial.section]);

  return (
    <FluxGlassRoot tier={selection.tier} reducedMotion={motionChoice === "system" ? undefined : reducedMotion}>
      <div className="lab" data-lab-marker={DESIGN_LAB_MARKER}>
        <aside className="lab-rail" aria-label="Design lab">
          <div className="lab-brand">
            <PegolesMark size={30} state="idle" decorative />
            <div>
              <p className="lab-brand__name">Flux Glass</p>
              <p className="lab-brand__meta">Design lab · dev only</p>
            </div>
          </div>

          <nav className="lab-nav" aria-label="Sections">
            <ol>
              {NAV.map((item, i) => (
                <li key={item.id}>
                  <button type="button" className="lab-nav__item" onClick={() => scrollToSection(item.id)}>
                    <span className="lab-nav__index">{String(i + 1).padStart(2, "0")}</span>
                    {item.label}
                  </button>
                </li>
              ))}
            </ol>
          </nav>

          <section className="lab-env" aria-labelledby="lab-env-title">
            <h2 id="lab-env-title" className="lab-env__title">
              Environment
            </h2>
            <div className="lab-env__group">
              <span className="lab-env__label">Effects</span>
              <Segmented
                label="Effects tier"
                value={tierChoice}
                onChange={setTierChoice}
                size="sm"
                options={[
                  { value: "auto", label: "Auto" },
                  { value: "full", label: "Full" },
                  { value: "reduced", label: "Reduced" },
                  { value: "minimal", label: "Minimal" },
                ]}
              />
            </div>
            <div className="lab-env__group">
              <span className="lab-env__label">Governor fixture</span>
              <Segmented
                label="Governor recommendation fixture"
                value={governorId}
                onChange={setGovernorId}
                size="sm"
                options={GOVERNOR_FIXTURES.map((g) => ({ value: g.id, label: g.label }))}
              />
              <span className="lab-env__hint">
                {governor?.machine} → {governor?.recommended ?? "no recommendation"}
              </span>
            </div>
            <div className="lab-env__group">
              <span className="lab-env__label">Motion</span>
              <Segmented
                label="Reduced motion"
                value={motionChoice}
                onChange={setMotionChoice}
                size="sm"
                options={[
                  { value: "system", label: "System" },
                  { value: "reduce", label: "Reduce" },
                  { value: "allow", label: "Allow" },
                ]}
              />
              <span className="lab-env__hint">System preference: {systemReducedMotion ? "reduce" : "no preference"}</span>
            </div>
            <Switch label="Low Power Mode" hint="Caps Full → Reduced" checked={lowPower} onChange={setLowPower} />
          </section>
        </aside>

        <main className="lab-main">
          <StatusStrip selection={selection} machine={governor?.machine ?? ""} />

          <header className="lab-intro">
            <p className="lab-intro__eyebrow">Pegoles · Phase 4</p>
            <h1 className="lab-intro__title">Flux Glass</h1>
            <p className="lab-intro__lead">
              A near-black operating environment lit by one presence. Blue means Pegoles is here — listening, working,
              focused. Everything else stays quiet, readable and cheap to draw.
            </p>
          </header>

          <ColorSection />
          <TypeSection />
          <MaterialsSection />
          <ButtonsSection />
          <StatusSection />
          <CommandSection />
          <ActivitySection />
          <ViewportSection initialState={initial.state} />
          <MotionSection />
          <EffectsSection />

          <div id="lab-presence" className="lab-section lab-section--external">
            <p className="lab-section__index">
              11<span className="lab-section__owner">Stream D2</span>
            </p>
            <PresenceSection />
          </div>
          <div id="lab-cursor" className="lab-section lab-section--external">
            <p className="lab-section__index">
              12<span className="lab-section__owner">Stream D2</span>
            </p>
            <AgentCursorSection />
          </div>
          <div id="lab-workspace" className="lab-section lab-section--external">
            <p className="lab-section__index">
              13<span className="lab-section__owner">Phase 4.1</span>
            </p>
            <WorkspaceSection />
          </div>
          <div id="lab-eyes-hands" className="lab-section lab-section--external">
            <p className="lab-section__index">
              14<span className="lab-section__owner">Phase 5</span>
            </p>
            <EyesHandsSection />
          </div>
        </main>
      </div>
    </FluxGlassRoot>
  );
}

export default DesignLab;

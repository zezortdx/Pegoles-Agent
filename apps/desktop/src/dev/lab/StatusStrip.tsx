import { useState } from "react";
import {
  effectsTiers,
  GlassSurface,
  StatusIndicator,
  useAmbientState,
  useBlurAudit,
  type EffectsSelection,
} from "@pegoles/ui";

const SOURCE_LABEL: Readonly<Record<EffectsSelection["source"], string>> = {
  user: "user override",
  governor: "governor",
  default: "default (governor pending)",
};

/**
 * Floating glass readout: resolved tier, ambient gate, blur budget.
 * The blur audit is the dev-only overlay the design system promises —
 * over-budget shows here, never as console noise.
 */
export function StatusStrip({ selection, machine }: { readonly selection: EffectsSelection; readonly machine: string }) {
  const ambient = useAmbientState();
  const audit = useBlurAudit();
  const [open, setOpen] = useState(false);
  const budget = effectsTiers[selection.tier].maxBackdropSurfaces;
  const over = audit.visible > budget;
  const params = effectsTiers[selection.tier];

  return (
    <div className="lab-strip-wrap">
      <GlassSurface as="div" radius="pill" elevation={3} className="lab-strip" auditLabel="Lab status strip">
        <span className="lab-strip__item">
          <span className="lab-strip__key">Tier</span>
          <span className="lab-strip__value">{params.label}</span>
          <span className="lab-strip__note">
            {SOURCE_LABEL[selection.source]}
            {selection.source === "governor" ? ` · ${machine}` : ""}
            {selection.lowPowerApplied ? " · Low Power" : ""}
          </span>
        </span>
        <span className="lab-strip__item">
          <span className="lab-strip__key">Motion</span>
          <span className="lab-strip__value">{selection.reducedMotion ? "Reduced" : "Full"}</span>
        </span>
        <span className="lab-strip__item">
          <span className="lab-strip__key">Ambient</span>
          <StatusIndicator
            tone={ambient?.running ? "active" : "paused"}
            label={ambient ? (ambient.running ? "Running" : `Paused · ${ambient.reasons[0] ?? ""}`) : "—"}
            size="sm"
          />
        </span>
        <button
          type="button"
          className="lab-strip__item lab-strip__budget"
          data-over={over ? "true" : "false"}
          aria-expanded={open}
          aria-controls="lab-blur-audit"
          onClick={() => setOpen((o) => !o)}
        >
          <span className="lab-strip__key">Blur</span>
          <StatusIndicator
            tone={over ? "danger" : "success"}
            label={`${audit.visible} / ${budget} visible`}
            detail={`${audit.mounted} mounted`}
            size="sm"
          />
        </button>
      </GlassSurface>
      {open && (
        <GlassSurface id="lab-blur-audit" className="lab-audit" elevation={3} radius="xl" auditLabel="Blur audit panel">
          <p className="lab-audit__title">
            Backdrop-filter surfaces {over ? "— over budget" : "— within budget"}
          </p>
          <ol className="lab-audit__list">
            {audit.surfaces.map((s) => (
              <li key={s.id} data-onscreen={s.onscreen ? "true" : "false"}>
                <span>{s.label}</span>
                <span className="lab-mono">
                  {s.material} · {s.blurPx}px · {s.onscreen ? "visible" : "offscreen"}
                </span>
              </li>
            ))}
            {audit.surfaces.length === 0 && <li>No backdrop blur on this tier.</li>}
          </ol>
        </GlassSurface>
      )}
    </div>
  );
}

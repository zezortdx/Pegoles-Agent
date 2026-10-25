import {
  EFFECTS_TIERS,
  effectsTiers,
  EffectsScope,
  GlassSurface,
  StatusIndicator,
  type EffectsTier,
  type EffectsTierParams,
} from "@pegoles/ui";
import { LabSection } from "../lab/controls";

interface Row {
  readonly label: string;
  readonly value: (p: EffectsTierParams) => string;
}

const ROWS: readonly Row[] = [
  { label: "Backdrop blur (regular · clear · electric)", value: (p) => `${p.glass.regular.blurPx} · ${p.glass.clear.blurPx} · ${p.glass.electric.blurPx} px` },
  { label: "Scrim opacity (regular)", value: (p) => `${Math.round(p.glass.regular.alpha * 100)}%` },
  { label: "Saturation", value: (p) => `${Math.round(p.saturate * 100)}%` },
  { label: "Glow intensity", value: (p) => p.glowIntensity.toFixed(2) },
  { label: "Specular edge", value: (p) => p.specular.toFixed(2) },
  { label: "Ambient life", value: (p) => (p.ambient.enabled ? `${p.ambient.periodMs / 1000} s · amplitude ${p.ambient.amplitude}` : "off") },
  { label: "Transitions", value: (p) => p.transition },
  { label: "Blur budget (visible surfaces)", value: (p) => String(p.maxBackdropSurfaces) },
];

const PREVIEW_LINES = [
  "guest runtime   ready    4.0 s",
  "weston          pixman   1440×900",
  "foot            pid 412  18 MB",
  "display         attached 5.6 s",
].join("\n");

function TierPreview({ tier }: { readonly tier: EffectsTier }) {
  const params = effectsTiers[tier];
  return (
    <EffectsScope tier={tier} className="lab-tier-preview">
      <div className="lab-tier-preview__stage" aria-hidden="true">
        <span className="lab-backdrop__orb" />
        <span className="lab-backdrop__core" />
        <pre className="lab-backdrop__text">{PREVIEW_LINES}</pre>
      </div>
      <GlassSurface material="regular" className="lab-tier-preview__card" auditLabel={`Tier preview · ${params.label}`}>
        <p className="lab-tier-preview__name">{params.label}</p>
        <StatusIndicator tone="active" label="Pegoles is working" pulse size="sm" />
        <p className="lab-tier-preview__summary">{params.summary}</p>
      </GlassSurface>
    </EffectsScope>
  );
}

export function EffectsSection() {
  return (
    <LabSection
      id="lab-effects"
      index="10"
      title="Effects tiers"
      lead="The ResourceGovernor recommends a tier from real hardware facts; the user can override it in Settings; reduced motion is a separate axis that never lowers richness — it only removes movement. Minimal still looks like Pegoles."
    >
      <div className="lab-tier-previews">
        {EFFECTS_TIERS.map((tier) => (
          <TierPreview key={tier} tier={tier} />
        ))}
      </div>
      <div className="lab-table-wrap">
        <table className="lab-table lab-table--tiers">
          <caption className="pg-visually-hidden">Effects tier parameters</caption>
          <thead>
            <tr>
              <th scope="col">Parameter</th>
              {EFFECTS_TIERS.map((tier) => (
                <th key={tier} scope="col">
                  {effectsTiers[tier].label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {ROWS.map((row) => (
              <tr key={row.label}>
                <th scope="row">{row.label}</th>
                {EFFECTS_TIERS.map((tier) => (
                  <td key={tier} className="lab-mono">
                    {row.value(effectsTiers[tier])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </LabSection>
  );
}

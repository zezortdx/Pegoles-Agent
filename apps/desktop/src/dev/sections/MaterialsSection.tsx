import { GlassSurface, glassMaterials, GLASS_MATERIALS, useFluxGlass, StatusIndicator } from "@pegoles/ui";
import { LabSection } from "../lab/controls";

const BACKDROP_LINES = [
  "pegoles-computer  ready  1440×900",
  "guest runtime     vsock  protocol 3",
  "weston            pixman renderer",
  "foot              pid 412   rss 18 MB",
  "display           attached  5.6 s",
];

/** Static scenery so the blur has something to act on. */
function Backdrop() {
  return (
    <div className="lab-backdrop" aria-hidden="true">
      <span className="lab-backdrop__orb" />
      <span className="lab-backdrop__core" />
      <pre className="lab-backdrop__text">{BACKDROP_LINES.join("\n")}</pre>
      <span className="lab-backdrop__grid" />
    </div>
  );
}

export function MaterialsSection() {
  const { params } = useFluxGlass();
  return (
    <LabSection
      id="lab-materials"
      index="03"
      title="Materials"
      lead="Glass is for layers that float over moving content. Structure stays solid. Blur depth and scrim come from the effects tier; the scrim is proven against the brightest light Pegoles emits, so text never drowns."
    >
      <div className="lab-materials">
        <Backdrop />
        <div className="lab-materials__cards">
          {GLASS_MATERIALS.map((material) => {
            const spec = glassMaterials[material];
            const recipe = params.glass[material];
            return (
              <GlassSurface
                key={material}
                material={material}
                elevation={2}
                className="lab-material"
                auditLabel={`Materials · ${spec.label}`}
              >
                <p className="lab-material__name">{spec.label}</p>
                <p className="lab-material__use">{spec.use}</p>
                <p className="lab-mono lab-material__recipe">
                  {recipe.blurPx > 0 ? `blur ${recipe.blurPx}px` : "no blur"} · scrim {Math.round(recipe.alpha * 100)}%
                </p>
                {material === "regular" && (
                  <GlassSurface material="clear" radius="pill" elevation={0} className="lab-material__nested">
                    <StatusIndicator tone="neutral" label="Nested: no second blur" size="sm" />
                  </GlassSurface>
                )}
              </GlassSurface>
            );
          })}
        </div>
      </div>
    </LabSection>
  );
}

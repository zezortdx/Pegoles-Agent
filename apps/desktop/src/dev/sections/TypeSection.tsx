import { typeScale, type TypeRoleName } from "@pegoles/ui";
import { LabSection } from "../lab/controls";

const SAMPLES: Readonly<Record<TypeRoleName, string>> = {
  display: "AI gets its own computer.",
  title1: "Pegoles Computer",
  title2: "Starting Pegoles Computer",
  headline: "Open a terminal and check disk space",
  body: "Pegoles works inside an isolated Debian computer. Nothing it does touches your files unless you hand them over.",
  callout: "Guest runtime ready in 4.0 s",
  footnote: "Your keyboard and pointer go to Pegoles Computer until you return.",
  caption: "Last frame kept while paused",
  label: "Startup stages",
  data: "1440 × 900 · vsock 3 · 412 MB of 1.1 GB",
};

export function TypeSection() {
  return (
    <LabSection
      id="lab-type"
      index="02"
      title="Typography"
      lead="The system face carries the interface — Pegoles sits beside native traffic lights and Liquid Glass, so it speaks the platform's voice with its optical sizes. Mono carries identity: labels, timings and dimensions read like instrument readouts."
    >
      <dl className="lab-type">
        {(Object.keys(typeScale) as TypeRoleName[]).map((role) => {
          const spec = typeScale[role];
          return (
            <div key={role} className="lab-type__row">
              <dt>
                <span className="lab-type__role">{role}</span>
                <span className="lab-mono">
                  {spec.size}/{spec.lineHeight} · {spec.weight} · {spec.tracking}em · {spec.family}
                </span>
              </dt>
              <dd className={`pg-type-${role}`}>{SAMPLES[role]}</dd>
            </div>
          );
        })}
      </dl>
    </LabSection>
  );
}

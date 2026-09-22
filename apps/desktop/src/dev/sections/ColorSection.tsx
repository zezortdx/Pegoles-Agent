import {
  composite,
  contrastRatio,
  hexToRgb,
  hueSaturation,
  palette,
  resolveColor,
  semantic,
  semanticEntries,
  semanticVar,
  signal,
  StatusIndicator,
  type PaletteToken,
  type SignalToken,
  type StatusTone,
} from "@pegoles/ui";
import { LabSection } from "../lab/controls";

const PALETTE_NOTES: Readonly<Record<PaletteToken, string>> = {
  void: "Canvas",
  surface: "Structure",
  blueDeep: "Depth",
  blueShadow: "Shade",
  pegolesBlue: "Presence",
  electric: "Energy",
  cyan: "Glow · focus",
  ice: "Eye light",
  text: "Text",
  muted: "Secondary",
};

const SIGNAL_TONES: Readonly<Record<SignalToken, { tone: StatusTone; role: string }>> = {
  green: { tone: "success", role: "Success" },
  amber: { tone: "warning", role: "Warning" },
  red: { tone: "danger", role: "Danger" },
  orchid: { tone: "waiting", role: "Waiting on you" },
};

const VOID = hexToRgb(palette.void);

function contrastOnVoid(path: string): string {
  const entry = semanticEntries().find((e) => e.path === path);
  if (!entry) return "—";
  const c = resolveColor(entry.token);
  const effective = c.alpha < 1 ? composite(c.rgb, c.alpha, VOID) : c.rgb;
  return `${contrastRatio(effective, VOID).toFixed(1)}:1`;
}

export function ColorSection() {
  const groups = Object.keys(semantic);
  return (
    <LabSection
      id="lab-color"
      index="01"
      title="Color"
      lead="Mostly void. Blue is never decoration: it means Pegoles is present, working, or focused. Status speaks in warm and green hues, always with a shape and a word."
    >
      <div className="lab-palette" role="list" aria-label="Raw palette">
        {(Object.keys(palette) as PaletteToken[]).map((key) => (
          <figure key={key} className="lab-chip" role="listitem">
            <span className="lab-chip__swatch" style={{ background: palette[key] }} />
            <figcaption>
              <span className="lab-chip__name">{key}</span>
              <span className="lab-chip__hex">{palette[key]}</span>
              <span className="lab-chip__note">{PALETTE_NOTES[key]}</span>
            </figcaption>
          </figure>
        ))}
      </div>

      <div className="lab-signal">
        {(Object.keys(signal) as SignalToken[]).map((key) => {
          const { hue } = hueSaturation(hexToRgb(signal[key]));
          return (
            <div key={key} className="lab-signal__item">
              <StatusIndicator tone={SIGNAL_TONES[key].tone} label={SIGNAL_TONES[key].role} pill />
              <span className="lab-mono">
                {signal[key]} · {Math.round(hue)}°
              </span>
            </div>
          );
        })}
        <p className="lab-footnote">
          Tested: no status hue falls in the blue band (165°–270°); every status color clears 4.5:1 on Void, Surface
          and Elevated.
        </p>
      </div>

      <div className="lab-table-wrap">
        <table className="lab-table">
          <caption className="pg-visually-hidden">Semantic color tokens</caption>
          <thead>
            <tr>
              <th scope="col">Token</th>
              <th scope="col">CSS variable</th>
              <th scope="col">Value</th>
              <th scope="col">On Void</th>
            </tr>
          </thead>
          {groups.map((group) => (
            <tbody key={group}>
              <tr className="lab-table__group">
                <th scope="rowgroup" colSpan={4}>
                  {group}
                </th>
              </tr>
              {semanticEntries()
                .filter((e) => e.path.startsWith(`${group}.`))
                .map(({ path, token }) => {
                  const color = resolveColor(token);
                  return (
                    <tr key={path}>
                      <th scope="row">
                        <span className="lab-swatch" style={{ background: color.css }} aria-hidden="true" />
                        {path}
                      </th>
                      <td className="lab-mono">{semanticVar(path)}</td>
                      <td className="lab-mono">{color.css}</td>
                      <td className="lab-mono">{contrastOnVoid(path)}</td>
                    </tr>
                  );
                })}
            </tbody>
          ))}
        </table>
      </div>
    </LabSection>
  );
}

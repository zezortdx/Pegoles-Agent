import { useEffect, type ReactNode, type Ref } from "react";
import type { ComputerModel } from "../state/computerModel";
import { modelConnected } from "../state/agentState";
import type { HostCapabilities, StatusPayload } from "../lib/tauri";
import { hostLabel, shortcutModifier } from "../lib/format";
import { SegmentedControl } from "../ui/SegmentedControl";

export type QualityChoice = "auto" | "full" | "reduced";
/** A section another surface points at (Home's safety line, the composer's model chip). */
export type SettingsAnchor = "security" | "pegoles";

export const QUALITY_SEGMENTS = [
  { value: "auto", label: "Auto" },
  { value: "full", label: "Full" },
  { value: "reduced", label: "Reduced" },
] as const satisfies readonly { value: QualityChoice; label: string }[];

export interface SettingsViewProps {
  readonly headingRef?: Ref<HTMLHeadingElement>;
  readonly connected: boolean;
  readonly native: boolean;
  readonly status: StatusPayload | null;
  readonly host: HostCapabilities | null;
  readonly computer: ComputerModel;
  readonly quality: QualityChoice;
  readonly resolvedQuality: "full" | "reduced";
  readonly onQuality: (value: QualityChoice) => void;
  readonly systemReducedMotion: boolean;
  readonly eventCount: number;
  /** Scroll to and focus this section on arrival. */
  readonly anchor?: SettingsAnchor | null;
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="setting" role="listitem">
      <div className="setting__text">
        <span className="setting__label">{label}</span>
        {hint && <span className="setting__hint">{hint}</span>}
      </div>
      <div className="setting__value">{children}</div>
    </div>
  );
}

/** A titled group of rows on one matte surface, macOS grouped-form style. */
function Section({ id, title, lead, note, children }: { id: string; title: string; lead?: string; note?: string; children: ReactNode }) {
  return (
    <section className="settings__section" aria-labelledby={id} data-section={id}>
      <div className="settings__head">
        <h2 id={id} className="settings__title" tabIndex={-1}>{title}</h2>
        {lead && <p className="settings__lead">{lead}</p>}
      </div>
      <div className="settings__group" role="list">{children}</div>
      {note && <p className="settings__note">{note}</p>}
    </section>
  );
}

/**
 * How Core's policy treats actions in this build. Core does not report its
 * rules over IPC yet, so this mirrors crates/pegoles-policy/src/engine.rs
 * (`evaluate`, `classify_shell`) and must change with it.
 */
const SECURITY_RULES: readonly { label: string; value: string; tone: "on" | "blocked" | "ask" }[] = [
  { label: "Computer isolation", value: "On", tone: "on" },
  { label: "Files on this Mac", value: "Blocked", tone: "blocked" },
  { label: "Typing passwords or keys", value: "Blocked", tone: "blocked" },
  { label: "Opening websites", value: "Asks first", tone: "ask" },
  { label: "Writing outside its workspace", value: "Asks first", tone: "ask" },
  { label: "Downloads, installs and system changes", value: "Asks first", tone: "ask" },
  { label: "Destructive commands, like erasing a disk", value: "Blocked", tone: "blocked" },
];

export function SettingsView(props: SettingsViewProps) {
  const { status, host, computer } = props;
  const modifier = shortcutModifier();
  const model = modelConnected(status);
  const { anchor } = props;
  useEffect(() => {
    if (!anchor) return;
    const heading = document.getElementById(`settings-${anchor}`);
    if (typeof heading?.scrollIntoView === "function") heading.scrollIntoView({ block: "start", behavior: "smooth" });
    heading?.focus({ preventScroll: true });
  }, [anchor]);
  return (
    <section className="page page--settings" aria-labelledby="settings-title">
      <header className="page__head">
        <div className="page__heading">
          <h1 id="settings-title" ref={props.headingRef} tabIndex={-1} className="page__title">Settings</h1>
        </div>
      </header>

      <Section id="settings-general" title="General" note="Tasks are kept while Pegoles is open and cleared when you quit.">
        <Row label="New task"><kbd className="kbd">{modifier} N</kbd></Row>
        <Row label="Search tasks and actions"><kbd className="kbd">{modifier} K</kbd></Row>
        <Row label="Show or hide the sidebar"><kbd className="kbd">{modifier} \</kbd></Row>
        <Row label="Show or hide its computer"><kbd className="kbd">{modifier} J</kbd></Row>
        <Row label="Step its computer back (Full → Focus → Side → closed)"><kbd className="kbd">Esc</kbd></Row>
        <Row label="Task history"><span className="setting__muted">This session</span></Row>
      </Section>

      <Section id="settings-appearance" title="Appearance">
        <Row label="Motion quality" hint={props.quality === "auto" ? `Auto is using ${props.resolvedQuality === "full" ? "Full" : "Reduced"} on this Mac.` : "Full renders Pegoles in 3D. Reduced keeps it light."}>
          <SegmentedControl id="quality" label="Motion quality" segments={QUALITY_SEGMENTS} value={props.quality} onChange={props.onQuality} />
        </Row>
        <Row label="Reduce motion" hint="Follows your system accessibility setting.">
          <span className="setting__muted">{props.systemReducedMotion ? "On" : "Off"}</span>
        </Row>
      </Section>

      <Section id="settings-pegoles" title="Pegoles" note={model ? undefined : "Without a model Pegoles can’t work on tasks by itself yet."}>
        <Row label="Model">
          <span className={model ? undefined : "setting__muted"}>{model ? status?.model : "Not connected"}</span>
        </Row>
        <Row label="Pegoles Core">
          <span className="setting__status"><span className="dot" data-tone={props.connected ? "done" : undefined} aria-hidden="true" />{props.connected ? "Connected" : props.native ? "Connecting…" : "Desktop app required"}</span>
        </Row>
      </Section>

      <Section id="settings-computer" title="Computer" note="Pegoles’ own isolated computer. It never shares your desktop.">
        <Row label="Status"><span>{computer.chip}</span></Row>
        {status && <Row label="System"><span className="mono">{status.spec_os} · {status.spec_arch}</span></Row>}
        {status && <Row label="Resources"><span className="mono">{status.spec_vcpus} CPU · {+(status.spec_ram_mb / 1024).toFixed(1)} GB</span></Row>}
        {status?.display_config && <Row label="Display"><span className="mono">{status.display_config.width_px} × {status.display_config.height_px}</span></Row>}
        <Row label="Runs on"><span>{host ? hostLabel(host.platform, host.architecture) : "—"}</span></Row>
        {host?.required_setup.length ? (
          <div className="setting setting--stack" role="listitem">
            <span className="setting__label">Setup needed</span>
            <ul className="setting__list">{host.required_setup.map((step) => <li key={step}>{step}</li>)}</ul>
          </div>
        ) : null}
      </Section>

      <Section
        id="settings-security" title="Security" lead="Built-in rules"
        note="Pegoles Core checks every action against these rules before it runs, so they don’t depend on the model behaving. They’re fixed in this build."
      >
        {SECURITY_RULES.map((rule) => (
          <Row key={rule.label} label={rule.label}>
            <span className="security-value" data-tone={rule.tone}>{rule.value}</span>
          </Row>
        ))}
        <Row label="What it types" hint="The audit log records that text was typed, never the text itself.">
          <span className="security-value" data-tone="on">Never logged</span>
        </Row>
      </Section>

      <details className="settings__section settings__dev">
        <summary className="settings__dev-summary"><h2 className="settings__title">Developer</h2></summary>
        <div className="settings__group" role="list">
          <Row label="Backend"><span className="mono">{status ? status.backend : "—"}</span></Row>
          <Row label="Events in memory"><span className="mono">{props.eventCount}</span></Row>
        </div>
      </details>
    </section>
  );
}

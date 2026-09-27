import { useEffect, type Ref } from "react";
import type { ComputerModel } from "../state/computerModel";
import type { ModelSettingsState } from "../state/useModelSettings";
import type { IntelligenceState } from "../state/useIntelligence";
import type { HostCapabilities, StatusPayload } from "../lib/tauri";
import { hostLabel, shortcutModifier } from "../lib/format";
import { SegmentedControl } from "../ui/SegmentedControl";
import { IntelligenceSection } from "./IntelligenceSection";
import { Row, Section } from "./settingsParts";
import { HOST } from "../lib/host";

export type QualityChoice = "auto" | "full" | "reduced";
/** A section another surface points at (Home's safety line, the composer's model chip, a task waiting for a model). */
export type SettingsAnchor = "security" | "intelligence";

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
  readonly intelligence: IntelligenceState;
  /** The cloud provider's key, model and effort. */
  readonly model: ModelSettingsState;
  readonly quality: QualityChoice;
  readonly resolvedQuality: "full" | "reduced";
  readonly onQuality: (value: QualityChoice) => void;
  readonly systemReducedMotion: boolean;
  readonly eventCount: number;
  /** Scroll to and focus this section on arrival. */
  readonly anchor?: SettingsAnchor | null;
}

/**
 * How Core's policy treats actions in this build. Core does not report its
 * rules over IPC yet, so this mirrors crates/pegoles-policy/src/engine.rs
 * (`evaluate`) and the computer-use action set in pegoles-protocol, and
 * must change with them.
 */
const SECURITY_RULES: readonly { label: string; value: string; tone: "on" | "blocked" }[] = [
  { label: "Computer isolation", value: "On", tone: "on" },
  { label: `Your ${HOST}’s files, apps and screen`, value: "No access", tone: "blocked" },
  { label: "Typing private keys", value: "Blocked", tone: "blocked" },
  { label: "Clicking, typing and scrolling on its computer", value: "Within safety limits", tone: "on" },
];

export function SettingsView(props: SettingsViewProps) {
  const { status, host, computer } = props;
  const modifier = shortcutModifier();
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
        <Row label="Pegoles Core">
          <span className="setting__status"><span className="dot" data-tone={props.connected ? "done" : undefined} aria-hidden="true" />{props.connected ? "Connected" : props.native ? "Connecting…" : "Desktop app required"}</span>
        </Row>
      </Section>

      <IntelligenceSection id="settings-intelligence" native={props.native} intelligence={props.intelligence} model={props.model} />

      <Section id="settings-appearance" title="Appearance">
        <Row label="Motion quality" hint={props.quality === "auto" ? `Auto is using ${props.resolvedQuality === "full" ? "Full" : "Reduced"} on this ${HOST}.` : "Full renders Pegoles in 3D. Reduced keeps it light."}>
          <SegmentedControl id="quality" label="Motion quality" segments={QUALITY_SEGMENTS} value={props.quality} onChange={props.onQuality} />
        </Row>
        <Row label="Reduce motion" hint="Follows your system accessibility setting.">
          <span className="setting__muted">{props.systemReducedMotion ? "On" : "Off"}</span>
        </Row>
      </Section>

      <Section id="settings-computer" title="Computer" note="Pegoles’ own isolated computer. It never shares your desktop.">
        <Row label="Status"><span>{computer.chip}</span></Row>
        {host?.required_setup.length ? (
          <div className="setting setting--stack" role="listitem">
            <span className="setting__label">Setup needed</span>
            <ul className="setting__list">{host.required_setup.map((step) => <li key={step}>{step}</li>)}</ul>
          </div>
        ) : null}
        <details className="details advanced">
          <summary>Advanced</summary>
          <div className="advanced__body">
            <div className="settings__group settings__group--dense" role="list" aria-label="Computer details">
              {status && <Row label="System"><span className="mono">{status.spec_os} · {status.spec_arch}</span></Row>}
              {status && <Row label="Resources"><span className="mono">{status.spec_vcpus} CPU · {+(status.spec_ram_mb / 1024).toFixed(1)} GB</span></Row>}
              {status?.display_config && <Row label="Display"><span className="mono">{status.display_config.width_px} × {status.display_config.height_px}</span></Row>}
              <Row label="Runs on"><span>{host ? hostLabel(host.platform, host.architecture) : "—"}</span></Row>
            </div>
          </div>
        </details>
      </Section>

      <Section
        id="settings-security" title="Security" lead="Built-in rules"
        note="Pegoles Core checks every action against these rules before it runs, so they don’t depend on the model behaving. They’re fixed in this build, and Pegoles can’t ask you for approval yet: anything the rules don’t allow is refused."
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

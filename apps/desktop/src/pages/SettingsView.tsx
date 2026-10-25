import type { CopyKey } from "../i18n/catalog";
import { usePreferences } from "../preferences/PreferencesProvider";
import { t, useLocale } from "../i18n";
import { useEffect, type Ref } from "react";
import type { ComputerModel } from "../state/computerModel";
import type { ModelSettingsState } from "../state/useModelSettings";
import type { IntelligenceState } from "../state/useIntelligence";
import type { HostCapabilities, StatusPayload } from "../lib/tauri";
import { hostLabel, shortcutModifier } from "../lib/format";
import { SegmentedControl } from "../ui/SegmentedControl";
import { IntelligenceSection } from "./IntelligenceSection";
import { Row, Section } from "./settingsParts";

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
const SECURITY_RULES: readonly { label: CopyKey; value: CopyKey; tone: "on" | "blocked" }[] = [
  { label: "Computer isolation", value: "On", tone: "on" },
  { label: "Your Mac’s files, apps and screen", value: "No access", tone: "blocked" },
  { label: "Typing private keys", value: "Blocked", tone: "blocked" },
  { label: "Clicking, typing and scrolling on its computer", value: "Within safety limits", tone: "on" },
];

export function SettingsView(props: SettingsViewProps) {
  useLocale();
  const { setCustomizerOpen } = usePreferences();
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
          <h1 id="settings-title" ref={props.headingRef} tabIndex={-1} className="page__title">{t("Settings")}</h1>
        </div>
      </header>

      <Section id="settings-general" title={t("General")} note={t("Tasks are kept while Pegoles is open and cleared when you quit.")}>
        <Row label={t("New task")}><kbd className="kbd">{modifier} N</kbd></Row>
        <Row label={t("Search tasks and actions")}><kbd className="kbd">{modifier} K</kbd></Row>
        <Row label={t("Show or hide the sidebar")}><kbd className="kbd">{modifier} \</kbd></Row>
        <Row label={t("Show or hide its computer")}><kbd className="kbd">{modifier} J</kbd></Row>
        <Row label={t("Step its computer back (Full → Focus → Side → closed)")}><kbd className="kbd">Esc</kbd></Row>
        <Row label={t("Task history")}><span className="setting__muted">{t("This session")}</span></Row>
        <Row label={t("Pegoles Core")}>
          <span className="setting__status"><span className="dot" data-tone={props.connected ? "done" : undefined} aria-hidden="true" />{props.connected ? t("Connected") : props.native ? t("Connecting…") : t("Desktop app required")}</span>
        </Row>
      </Section>

      <IntelligenceSection id="settings-intelligence" native={props.native} intelligence={props.intelligence} model={props.model} />

      <Section id="settings-agent" title={t("Appearance preferences")}>
        <Row label={t("Customize your agent")}>
          <button className="btn" type="button" onClick={() => setCustomizerOpen(true)}>{t("Make your agent")}</button>
        </Row>
      </Section>

      <Section id="settings-appearance" title={t("Appearance")}>
        <Row label={t("Motion quality")} hint={props.quality === "auto" ? t("Auto is using {0} on this Mac.", { "0": props.resolvedQuality === "full" ? t("Full") : t("Reduced") }) : t("Full adds deeper glass, glow and motion. Reduced keeps it light.")}>
          <SegmentedControl id="quality" label={t("Motion quality")} segments={QUALITY_SEGMENTS.map((segment) => ({ ...segment, label: t(segment.label) }))} value={props.quality} onChange={props.onQuality} />
        </Row>
        <Row label={t("Reduce motion")} hint={t("Follows your system accessibility setting.")}>
          <span className="setting__muted">{props.systemReducedMotion ? t("On") : t("Off")}</span>
        </Row>
      </Section>

      <Section id="settings-computer" title={t("Computer")} note={t("Pegoles’ own isolated computer. It never shares your desktop.")}>
        <Row label={t("Status")}><span>{computer.chip}</span></Row>
        {status && <Row label={t("System")}><span className="mono">{status.spec_os} · {status.spec_arch}</span></Row>}
        {status && <Row label={t("Resources")}><span className="mono">{status.spec_vcpus} CPU · {+(status.spec_ram_mb / 1024).toFixed(1)} GB</span></Row>}
        {status?.display_config && <Row label={t("Display")}><span className="mono">{status.display_config.width_px} × {status.display_config.height_px}</span></Row>}
        <Row label={t("Runs on")}><span>{host ? hostLabel(host.platform, host.architecture) : "—"}</span></Row>
        {host?.required_setup.length ? (
          <div className="setting setting--stack" role="listitem">
            <span className="setting__label">{t("Setup needed")}</span>
            <ul className="setting__list">{host.required_setup.map((step) => <li key={step}>{step}</li>)}</ul>
          </div>
        ) : null}
      </Section>

      <Section
        id="settings-security" title={t("Security")} lead={t("Built-in rules")}
        note={t("Pegoles Core checks every action against these rules before it runs, so they don’t depend on the model behaving. They’re fixed in this build, and Pegoles can’t ask you for approval yet: anything the rules don’t allow is refused.")}
      >
        {SECURITY_RULES.map((rule) => (
          <Row key={rule.label} label={t(rule.label)}>
            <span className="security-value" data-tone={rule.tone}>{t(rule.value)}</span>
          </Row>
        ))}
        <Row label={t("What it types")} hint={t("The audit log records that text was typed, never the text itself.")}>
          <span className="security-value" data-tone="on">{t("Never logged")}</span>
        </Row>
      </Section>

      <details className="settings__section settings__dev">
        <summary className="settings__dev-summary"><h2 className="settings__title">{t("Developer")}</h2></summary>
        <div className="settings__group" role="list">
          <Row label={t("Backend")}><span className="mono">{status ? status.backend : "—"}</span></Row>
          <Row label={t("Events in memory")}><span className="mono">{props.eventCount}</span></Row>
        </div>
      </details>
    </section>
  );
}

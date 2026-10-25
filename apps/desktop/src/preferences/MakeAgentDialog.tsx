import { createPortal } from "react-dom";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { PresenceAvatar } from "../presence/PresenceAvatar";
import { SHAPES, type ShapeId } from "../presence/engine/skins";
import { EXPRESSIONS, type ExpressionId } from "../presence/engine/expressions";
import type { PresenceMode } from "../presence/modes";
import { t, useLocale } from "../i18n";
import type { CopyKey } from "../i18n/catalog";
import { AVATAR_COLORS, type AvatarProfile, type Locale } from "./types";
import { usePreferences } from "./PreferencesProvider";
import "./preferences.css";

const SHAPE_LABEL: Record<ShapeId, CopyKey> = { circle: "Circle", pebble: "Pebble", squircle: "Squircle", capsule: "Capsule", triangle: "Triangle", hexagon: "Hexagon", cloud: "Cloud", droplet: "Droplet" };
const EXPRESSION_LABEL: Record<ExpressionId, CopyKey> = { neutral: "Neutral", attentive: "Attentive", surprised: "Surprised", excited: "Excited", happy: "Happy", laughing: "Laughing", angry: "Angry", sad: "Sad", afraid: "Afraid", suspicious: "Suspicious", confused: "Confused", curious: "Curious", proud: "Proud", shy: "Shy", bored: "Bored", sleepy: "Sleepy" };
const COLOR_LABEL = { silver: "Silver", graphite: "Graphite", pearl: "Pearl" } as const;
const PREVIEW: readonly { mode: PresenceMode; label: CopyKey }[] = [
  { mode: "idle", label: "Idle" }, { mode: "thinking", label: "Thinking" }, { mode: "working", label: "Working" }, { mode: "needs-user", label: "Needs you" }, { mode: "error", label: "Problem" },
];
function Choices<T extends string>({ label, options, value, onChange, disabled }: {
  readonly label: string; readonly options: readonly { value: T; label: string }[]; readonly value: T;
  readonly onChange: (value: T) => void; readonly disabled: boolean;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const index = options.findIndex((option) => option.value === value);
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    const step = ["ArrowRight", "ArrowDown"].includes(event.key) ? 1 : ["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 0;
    const edge = event.key === "Home" ? 0 : event.key === "End" ? options.length - 1 : null;
    if (disabled || (!step && edge === null)) return;
    event.preventDefault();
    const next = edge ?? (index + step + options.length) % options.length;
    onChange(options[next].value); refs.current[next]?.focus();
  };
  return <fieldset className="agent-choices" disabled={disabled}><legend>{label}</legend>
    <div role="radiogroup" aria-label={label} onKeyDown={key}>
      {options.map((option, i) => <button key={option.value} ref={(el) => { refs.current[i] = el; }} type="button" role="radio" aria-checked={option.value === value}
        tabIndex={i === index ? 0 : -1} disabled={disabled} onClick={() => onChange(option.value)}>{option.label}</button>)}
    </div>
  </fieldset>;
}
export function MakeAgentDialog({ onClose }: { readonly onClose: () => void }) {
  useLocale();
  const { preferences, ready, loading, loadError, saving, reload, save, setCustomizerOpen } = usePreferences();
  const [profile, setProfile] = useState(preferences.avatar);
  const [locale, setLocale] = useState<Locale>(preferences.locale);
  const [preview, setPreview] = useState<PresenceMode>("idle");
  const [error, setError] = useState<"name" | "save" | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const name = useRef<HTMLInputElement>(null);
  const saveButton = useRef<HTMLButtonElement>(null);
  const [loadedDraft, setLoadedDraft] = useState(ready);
  const draftReady = ready && loadedDraft;
  const id = useId();
  // Opening before the initial read completes must never edit a stale default.
  useEffect(() => {
    if (ready && !loadedDraft) { setProfile(preferences.avatar); setLocale(preferences.locale); setLoadedDraft(true); }
  }, [ready, preferences, loadedDraft]);
  useEffect(() => {
    setCustomizerOpen(true);
    const layer = root.current?.parentElement;
    const siblings = Array.from(document.body.children).filter((element) => element !== layer);
    const previous = siblings.map((element) => ({ element, inert: element.hasAttribute("inert"), hidden: element.getAttribute("aria-hidden") }));
    for (const { element } of previous) { element.setAttribute("inert", ""); element.setAttribute("aria-hidden", "true"); }
    const contain = (event: FocusEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) root.current?.focus();
    };
    document.addEventListener("focusin", contain);
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    name.current?.focus();
    return () => {
      document.removeEventListener("focusin", contain);
      for (const { element, inert, hidden } of previous) {
        if (!inert) element.removeAttribute("inert");
        if (hidden === null) element.removeAttribute("aria-hidden"); else element.setAttribute("aria-hidden", hidden);
      }
      setCustomizerOpen(false);
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [setCustomizerOpen]);
  useEffect(() => { if (draftReady) name.current?.focus(); }, [draftReady]);
  useEffect(() => { if (error === "save" && !saving) saveButton.current?.focus(); }, [error, saving]);
  const change = <K extends keyof AvatarProfile>(key: K, value: AvatarProfile[K]) => { setProfile((previous) => ({ ...previous, [key]: value })); setError(null); };
  const valid = profile.name.trim().length > 0 && Array.from(profile.name.trim()).length <= 32 && !Array.from(profile.name).some((character) => { const point = character.codePointAt(0) ?? 0; return point < 32 || (point >= 127 && point <= 159) || /[\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/u.test(character); });
  const submit = async () => {
    if (!valid) { setError("name"); name.current?.focus(); return; }
    setError(null);
    try { await save({ ...preferences, avatar: profile, locale }); onClose(); }
    catch { setError("save"); }
  };
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!saving) onClose(); }
    if (event.key !== "Tab") return;
    const stops = Array.from(root.current?.querySelectorAll<HTMLElement>('button:not(:disabled):not([tabindex="-1"]), input:not(:disabled), select:not(:disabled), [tabindex="0"]') ?? []);
    const first = stops[0]; const last = stops[stops.length - 1];
    if (!first) { event.preventDefault(); root.current?.focus(); }
    else if (event.shiftKey && (document.activeElement === first || !root.current?.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || !root.current?.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
  };
  return createPortal(<div className="agent-dialog-layer">
    <div ref={root} className="agent-dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-title`} aria-describedby={`${id}-intro`} tabIndex={-1} onKeyDown={key} aria-busy={saving || loading}>
      <form onSubmit={(event) => { event.preventDefault(); if (draftReady && !saving) void submit(); }}>
        <h2 id={`${id}-title`}>{t("Make your agent")}</h2>
        <p id={`${id}-intro`}>{t("Choose a look and a name. Your agent still follows the same safety rules.")}</p>
        <label htmlFor={`${id}-name`}>{t("Agent name")}</label>
        <input id={`${id}-name`} ref={name} value={profile.name} disabled={saving || !draftReady} autoComplete="off" maxLength={128}
          aria-invalid={error === "name" || undefined} aria-describedby={`${id}-name-hint${error ? ` ${id}-error` : ""}`} onChange={(event) => change("name", event.target.value)} />
        <small id={`${id}-name-hint`}>{t("Use 1 to 32 characters, without control or direction marks.")}</small>
        <div className="agent-dialog__preview">
          <PresenceAvatar mode={preview} size={144} profile={profile} interactive label={t("Preview of {name}", { name: profile.name.trim() || preferences.avatar.name })} />
          <span>{profile.name.trim()}</span>
        </div>
        <Choices label={t("Preview state")} options={PREVIEW.map((entry) => ({ value: entry.mode, label: t(entry.label) }))} value={preview} onChange={setPreview} disabled={saving || !draftReady} />
        <Choices label={t("Shape")} options={SHAPES.map(({ id }) => ({ value: id, label: t(SHAPE_LABEL[id]) }))} value={profile.shape} onChange={(value) => change("shape", value)} disabled={saving || !draftReady} />
        <Choices label={t("Color")} options={AVATAR_COLORS.map((value) => ({ value, label: t(COLOR_LABEL[value]) }))} value={profile.color} onChange={(value) => change("color", value)} disabled={saving || !draftReady} />
        <Choices label={t("Expression")} options={EXPRESSIONS.map(({ id }) => ({ value: id, label: t(EXPRESSION_LABEL[id]) }))} value={profile.expression} onChange={(value) => change("expression", value)} disabled={saving || !draftReady} />
        <label htmlFor={`${id}-language`}>{t("Language")}</label>
        <select id={`${id}-language`} value={locale} disabled={saving || !draftReady} onChange={(event) => setLocale(event.target.value as Locale)}>
          <option value="en">English</option><option value="pt-BR">Português (Brasil)</option>
        </select>
        {loading && <p role="status">{t("Loading…")}</p>}
        {(error || loadError) && <p id={`${id}-error`} role="alert">{t(loadError ? "Could not load preferences. Try again." : error === "name" ? "Use 1 to 32 characters, without control or direction marks." : "Could not save your agent. Try again.")}</p>}
        {loadError && <button type="button" disabled={loading} onClick={() => { void reload(); }}>{t("Try again")}</button>}
        <footer><button type="button" disabled={saving} onClick={onClose}>{t("Cancel")}</button>
          <button ref={saveButton} type="submit" disabled={saving || !draftReady}>{t(saving ? "Saving…" : "Save")}</button></footer>
      </form>
    </div>
  </div>, document.body);
}

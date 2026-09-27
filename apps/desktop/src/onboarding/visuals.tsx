import type { ReactNode } from "react";
import { PresenceMark } from "../presence";
import { AlertIcon, CheckIcon, ComputerIcon, PersonIcon, ShieldIcon } from "../ui/icons";
import type { CheckTone } from "./systemCheck";

/**
 * Onboarding's small pictures. Static SVG/CSS only (no WebGL here), so
 * the first screens paint instantly on modest hardware; motion is CSS and
 * stops under Reduce Motion.
 */

/** You → Pegoles → its own computer, with the boundary drawn around the computer. */
export function FlowDiagram({ computer }: { readonly computer: string }) {
  return (
    <figure className="ob-flow" aria-label={`You give Pegoles a task. Pegoles does it on its own isolated computer, not on your ${computer}.`}>
      <Node icon={<PersonIcon size={20} />} label="You" note="describe a task" />
      <Link />
      <Node icon={<PresenceMark mode="idle" size={22} decorative />} label="Pegoles" note="plans each step" accent />
      <Link />
      <div className="ob-flow__fence">
        <span className="ob-flow__fence-label"><ShieldIcon size={12} />Isolated</span>
        <Node icon={<ComputerIcon size={20} />} label="Its own computer" note="where the work happens" />
      </div>
    </figure>
  );
}

function Node({ icon, label, note, accent }: { readonly icon: ReactNode; readonly label: string; readonly note: string; readonly accent?: boolean }) {
  return (
    <div className="ob-flow__node" data-accent={accent || undefined}>
      <span className="ob-flow__icon" aria-hidden="true">{icon}</span>
      <span className="ob-flow__label">{label}</span>
      <span className="ob-flow__note">{note}</span>
    </div>
  );
}

function Link() {
  return <span className="ob-flow__link" aria-hidden="true"><span className="ob-flow__pulse" /></span>;
}

const TONE_ICON: Record<CheckTone, ReactNode> = {
  ok: <CheckIcon size={14} />,
  warn: <AlertIcon size={14} />,
  action: <AlertIcon size={14} />,
  blocked: <AlertIcon size={14} />,
};

/** A check row's state: an icon and a shape, never colour alone. */
export function ToneGlyph({ tone }: { readonly tone: CheckTone | "pending" }) {
  return (
    <span className="ob-glyph" data-tone={tone} aria-hidden="true">
      {tone === "pending" ? <span className="ob-glyph__spin" /> : TONE_ICON[tone]}
    </span>
  );
}

/** Pegoles' computer being prepared: a small screen with a light passing through it. */
export function MachineGlyph({ state }: { readonly state: "idle" | "working" | "done" | "failed" }) {
  return (
    <div className="ob-machine" data-state={state} aria-hidden="true">
      <div className="ob-machine__screen">
        <span className="ob-machine__scan" />
        <span className="ob-machine__mark"><PresenceMark mode={state === "done" ? "done" : state === "failed" ? "error" : "idle"} size={26} decorative /></span>
      </div>
      <div className="ob-machine__stand" />
    </div>
  );
}

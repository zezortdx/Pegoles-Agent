import { useEffect, useId, useRef, useState } from "react";
import type { PendingApproval } from "../lib/tauri";
import type { CopyKey } from "../i18n/catalog";
import { t, useLocale } from "../i18n";

const summaryKeys: Readonly<Record<string, CopyKey>> = {
  "VM input: type": "Type text in the VM",
  "VM input: move": "Move the pointer in the VM",
  "VM input: click": "Click in the VM",
  "VM input: double_click": "Double-click in the VM",
  "VM input: mouse_down": "Press the pointer in the VM",
  "VM input: mouse_up": "Release the pointer in the VM",
  "VM input: drag": "Drag in the VM",
  "VM input: scroll": "Scroll in the VM",
  "VM input: key_press": "Press a key in the VM",
  "VM input: key_chord": "Press a key combination in the VM",
};
function safeSummary(summary: string) {
  return Object.hasOwn(summaryKeys, summary) ? t(summaryKeys[summary]) : summary;
}

export function ApprovalCard({ approval, resolve }: { approval: PendingApproval; resolve: (id: string, decision: "approve_once" | "deny") => Promise<void> }) {
  useLocale();
  const heading = useId();
  const [now, setNow] = useState(Date.now);
  const [pending, setPending] = useState(false);
  const [done, setDone] = useState(false);
  const [failed, setFailed] = useState(false);
  const status = useRef<HTMLParagraphElement>(null);
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  const remaining = Math.max(0, Math.ceil((Date.parse(approval.expires_at) - now) / 1000));
  const expired = !Number.isFinite(remaining) || remaining === 0;
  const decide = async (decision: "approve_once" | "deny") => {
    if (pending || done || expired) return;
    setPending(true); setFailed(false);
    try { await resolve(approval.id, decision); setDone(true); }
    catch { setFailed(true); }
    finally { setPending(false); status.current?.focus(); }
  };
  return <article className="ask" aria-labelledby={heading} aria-busy={pending}>
    <h3 id={heading}>{t("Needs your approval")}</h3>
    <p className="ask__reason">{safeSummary(approval.summary)}</p>
    <p><time dateTime={approval.expires_at}>{t("Expires in {0}s", { 0: remaining })}</time></p>
    <p ref={status} tabIndex={-1} role="status" aria-live="polite">
      {expired ? t("Expired · action denied") : pending ? t("Waiting for native confirmation…") : done ? t("Decision confirmed by Core.") : t("Approval applies only to this action.")}
    </p>
    {failed && <p role="alert">{t("The decision could not be confirmed.")}</p>}
    <div className="ask__actions">
      <button className="btn btn--line" type="button" disabled={pending || done || expired} onClick={() => void decide("deny")}>{t("Deny")}</button>
      <button className="btn btn--line" type="button" disabled={pending || done || expired} onClick={() => void decide("approve_once")}>{t("Approve once")}</button>
    </div>
  </article>;
}

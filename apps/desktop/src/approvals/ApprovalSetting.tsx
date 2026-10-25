import { useState } from "react";
import { api } from "../lib/tauri";
import { t, useLocale } from "../i18n";
export function ApprovalSetting({ enabled, refresh }: { enabled: boolean; refresh: () => Promise<void> }) {
  useLocale();
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const change = async () => {
    setPending(true); setFailed(false);
    try { await api.setInputApproval(!enabled); await refresh(); }
    catch { setFailed(true); }
    finally { setPending(false); }
  };
  return <div className="ask">
    <label><input type="checkbox" checked={enabled} disabled={pending} onChange={() => void change()} /> {t("Ask before agent input")}</label>
    <p>{t("This session only · resets to off when Pegoles restarts.")}</p>
    {pending && <p role="status">{t("Waiting for native confirmation…")}</p>}
    {failed && <p role="alert">{t("The decision could not be confirmed.")}</p>}
  </div>;
}

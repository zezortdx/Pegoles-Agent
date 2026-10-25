import { useEffect, useState } from "react";
import { t, useLocale } from "../i18n";
import type { CopyKey } from "../i18n/catalog";
import { api, type ArtifactData, type TaskHistory } from "../lib/tauri";
import { timeOf } from "./format";
import "./taskArtifacts.css";

const STATUS: Record<string, CopyKey> = { pending:"Not started", running:"Working", waiting_for_approval:"Needs approval", completed:"Done", failed:"Didn’t finish", cancelled:"Cancelled" };

/** Defense in depth for the fixed Core response. No paths, links or markup sinks. */
function valid(data: TaskHistory | null, id: string): boolean {
  if (data === null) return true;
  if (!data || data.task?.id !== id || !Array.isArray(data.audit) || data.audit.length > 32 || typeof data.recovered !== "boolean") return false;
  if (data.audit.some((row) => !Object.hasOwn(STATUS, row.status) || typeof row.at !== "string")) return false;
  if (!data.artifact) return true;
  if (data.artifact.task_id !== id) return false;
  const a = data.artifact.data;
  if (a?.kind === "text") return typeof a.text === "string" && [...a.text].length <= 4000;
  if (a?.kind !== "table" || !Array.isArray(a.columns) || !Array.isArray(a.rows) || a.columns.length < 1 || a.columns.length > 8 || a.rows.length < 1 || a.rows.length > 32) return false;
  if (a.rows.some((row) => !Array.isArray(row) || row.length !== a.columns.length)) return false;
  const cells = [...a.columns, ...a.rows.flat()];
  return cells.every((cell) => typeof cell === "string" && [...cell].length <= 256) && cells.reduce((sum, cell) => sum + [...cell].length, 0) <= 4000;
}
function Preview({ data }: { data: ArtifactData }) {
  if (data.kind === "text") return <pre className="task-artifacts__text">{data.text}</pre>;
  return <div className="task-artifacts__table"><table>
    <thead><tr>{data.columns.map((cell, index) => <th scope="col" key={index}>{cell}</th>)}</tr></thead>
    <tbody>{data.rows.map((row, index) => <tr key={index}>{row.map((cell, index) => <td key={index}>{cell}</td>)}</tr>)}</tbody>
  </table></div>;
}
export function TaskArtifacts({ taskId, revision, enabled }: { taskId: string; revision: string; enabled: boolean }) {
  useLocale();
  const key = `${taskId}:${revision}`;
  const [loaded, setLoaded] = useState<{ key: string; data: TaskHistory | null; error?: boolean } | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    void api.getTaskHistory(taskId).then((data) => {
      if (!disposed) setLoaded({ key, data: valid(data, taskId) ? data : null, error: !valid(data, taskId) });
    }).catch(() => { if (!disposed) setLoaded({ key, data:null, error:true }); });
    return () => { disposed = true; };
  }, [taskId, key, enabled]);
  const current = loaded?.key === key ? loaded : null;
  const data = current?.data;
  const result = data?.artifact;
  return <section className="task-artifacts" aria-label={t("Task results")}>
    <h2>{t("Task results")}</h2>
    {!enabled ? <p>{t("Connect to Pegoles Core to view saved results.")}</p> : !current ? <p role="status">{t("Loading saved results…")}</p> : current.error ? <p role="alert">{t("Could not load saved results.")}</p> : <>
      {data?.recovered && <p>{t("Closed before completion. This task won’t resume automatically.")}</p>}
      {result ? <><p className="task-artifacts__label">{result.data.kind === "text" ? t("Agent summary · text result") : t("Agent summary · table result")}</p><Preview data={result.data} /></> : <p>{t("No saved result for this task.")}</p>}
      {!!data?.audit.length && <details><summary>{t("Saved task history")}</summary><ol>{data.audit.map((entry, index) => <li key={index}>{t(STATUS[entry.status])} · {timeOf(entry.at)}</li>)}</ol></details>}
    </>}
  </section>;
}

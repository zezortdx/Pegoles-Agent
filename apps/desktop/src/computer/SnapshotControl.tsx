import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { api, type HumanAuthority, type HumanInput, type FrameMetaWire } from "../lib/tauri";
import { t, useLocale } from "../i18n";

type Box = { left: number; top: number; width: number; height: number };
export function guestPoint(box: Box, width: number, height: number, x: number, y: number) {
  if (![box.left, box.top, box.width, box.height, width, height, x, y].every(Number.isFinite) || Math.min(box.width, box.height, width, height) <= 0) return null;
  const scale = Math.min(box.width / width, box.height / height);
  const nx = (x - box.left - (box.width - width * scale) / 2) / (width * scale);
  const ny = (y - box.top - (box.height - height * scale) / 2) / (height * scale);
  return nx >= 0 && nx <= 1 && ny >= 0 && ny <= 1 ? { x: nx, y: ny } : null;
}
type Meta = Pick<FrameMetaWire, "frame_id" | "computer_id" | "width_px" | "height_px">;
export function SnapshotControl({ src, meta, generation, enabled }: { src: string; meta: Meta; generation: number; enabled: boolean }) {
  useLocale();
  const active = useRef(false);
  const queue = useRef<{ authority: HumanAuthority; input: HumanInput }[]>([]);
  const sending = useRef(false);
  const moved = useRef(0);
  const [issue, setIssue] = useState(false);
  const live = useRef({ enabled, generation });
  live.current = { enabled, generation };
  const release = () => {
    queue.current = [];
    if (!active.current) return;
    active.current = false;
    void api.returnControl().catch(() => setIssue(true));
  };
  useEffect(() => {
    active.current = enabled;
    const hidden = () => { if (document.hidden) release(); };
    window.addEventListener("blur", release);
    document.addEventListener("visibilitychange", hidden);
    return () => { release(); window.removeEventListener("blur", release); document.removeEventListener("visibilitychange", hidden); };
  }, [enabled, generation]);
  const send = (input: HumanInput) => {
    if (!live.current.enabled || !active.current || document.hidden) return;
    if (queue.current.length >= 16) { setIssue(true); release(); return; }
    queue.current.push({ authority: { computer_id: meta.computer_id, frame_id: meta.frame_id, width: meta.width_px, height: meta.height_px, generation }, input });
    if (sending.current) return;
    sending.current = true;
    void (async () => {
      try {
        while (active.current && live.current.enabled && queue.current.length) {
          const next = queue.current.shift();
          if (!next || next.authority.generation !== live.current.generation) break;
          await api.humanInput(next.authority, next.input);
        }
      } catch { setIssue(true); release(); }
      finally { sending.current = false; }
    })();
  };
  const text = (value: string) => {
    if ([...value].length > 256) { setIssue(true); return; }
    if (value) send({ type: "text", text: value });
  };
  const keyboard = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!enabled) return;
    if (e.ctrlKey && e.altKey && e.key === "Escape") { e.preventDefault(); release(); return; }
    // Paste comes only from the person's explicit browser paste event.
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "v") return;
    e.preventDefault();
    if (e.nativeEvent.isComposing || e.key === "Process" || e.key === "Dead") { setIssue(true); return; }
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
    const key = e.key === " " ? "Space" : e.key;
    const modifiers = [e.ctrlKey && "Control", e.altKey && "Alt", e.metaKey && "Meta", e.shiftKey && "Shift"].filter((v): v is string => !!v);
    if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) text(e.key);
    else if (modifiers.length) send({ type: "chord", keys: [...modifiers, key] });
    else send({ type: "key", key });
  };
  const point = (element: HTMLDivElement, x: number, y: number) => guestPoint(element.getBoundingClientRect(), meta.width_px, meta.height_px, x, y);
  const button = (value: number) => value === 2 ? "secondary" as const : value === 1 ? "middle" as const : "primary" as const;
  return <div className="snapshot-control-wrap">
    <div className="snapshot-control" role="application" aria-label={t("Guest screen controls")} aria-disabled={!enabled} tabIndex={enabled ? 0 : -1}
      onBlur={release} onContextMenu={(e) => { if (enabled) e.preventDefault(); }}
      onKeyDown={keyboard} onKeyUp={(e) => { if (enabled) e.preventDefault(); }}
      onCompositionStart={(e) => { e.preventDefault(); setIssue(true); }}
      onPaste={(e) => { if (!enabled) return; e.preventDefault(); text(e.clipboardData.getData("text/plain")); }}
      onPointerDown={(e) => { if (!enabled) return; e.preventDefault(); e.currentTarget.focus(); const p = point(e.currentTarget, e.clientX, e.clientY); if (p) { e.currentTarget.setPointerCapture?.(e.pointerId); send({ type: "down", ...p, button: button(e.button) }); } }}
      onPointerMove={(e) => { if (!enabled || Date.now() - moved.current < 50) return; moved.current = Date.now(); const p = point(e.currentTarget, e.clientX, e.clientY); if (p) send({ type: "move", ...p }); }}
      onPointerUp={(e) => { if (!enabled) return; const p = point(e.currentTarget, e.clientX, e.clientY); if (p) send({ type: "up", ...p, button: button(e.button) }); else release(); }}
      onPointerCancel={release}
      onWheel={(e) => { if (!enabled) return; const p = point(e.currentTarget, e.clientX, e.clientY); if (p) send({ type: "scroll", ...p, dx: Math.sign(e.deltaX) * 3, dy: Math.sign(e.deltaY) * 3 }); }}>
      <img className="screen-snapshot" src={src} alt={t("Latest snapshot of Pegoles’ computer screen")} draggable={false} />
    </div>
    {enabled && <p className="snapshot-control__hint">{t("Focus the guest screen to use it. Ctrl+Alt+Esc gives control back. Paste up to 256 characters; IME composition is unavailable.")}</p>}
    {issue && <p role="alert">{t("Input was not accepted. Refresh the screen and take control again.")}</p>}
  </div>;
}

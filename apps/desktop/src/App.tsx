import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LayoutGroup } from "motion/react";
import {
  AgentCursorOverlay, CommandBar, CommandMorph, FluxGlassRoot, GlassButton, GlassSurface, PegolesMark, StatusIndicator,
  MonitorIcon, PresenceIcon, TaskController, TaskIcon, WindowIcon, ProgressLine, VIEWPORT_STATE_SPECS,
  isEffectsTier, usePresence, type EffectsTier,
} from "@pegoles/ui";
import { ActivityTimeline } from "./components/ActivityTimeline";
import { ComputerControls } from "./components/ComputerControls";
import { NativeComputer } from "./components/NativeComputer";
import { TaskWorkspace, type WorkspaceLayout } from "./components/TaskWorkspace";
import { actionCursorSource } from "./lib/agentCursorFeed";
import { describeEvent } from "./lib/events";
import { api, type AgentTask, type AgentEvent, type HostCapabilities, type ImageStatusPayload, type StatusPayload, type BootLogPayload } from "./lib/tauri";

type Page = "Home" | "Agents" | "Computer" | "Activity" | "Settings";
const pages: Page[] = ["Home", "Agents", "Computer", "Activity", "Settings"];
const taskLabel = (task: AgentTask) => task.status === "pending" ? "Pending · execution unavailable" : task.status.replaceAll("_", " ");
function readEffects(): EffectsTier | "auto" {
  try { const value = localStorage.getItem("pegoles.effects"); return isEffectsTier(value) ? value : "auto"; } catch { return "auto"; }
}
function NavIcon({ page }: { page: Page }) {
  if (page === "Computer") return <MonitorIcon size={19} />;
  if (page === "Agents") return <PresenceIcon size={19} />;
  if (page === "Activity") return <TaskIcon size={19} />;
  return <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
    {page === "Home" ? <path d="m3 10 9-7 9 7v10H15v-7H9v7H3Z" strokeLinejoin="round" /> : <><path d="M4 6h16M4 12h16M4 18h16" /><circle cx="8" cy="6" r="2" fill="currentColor" /><circle cx="16" cy="12" r="2" fill="currentColor" /><circle cx="10" cy="18" r="2" fill="currentColor" /></>}
  </svg>;
}

function elapsedSince(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  if (!Number.isFinite(ms) || ms < 0) return "0:00";
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export default function App() {
  const native = isTauri();
  const [page, setPage] = useState<Page>("Home");
  const [layout, setLayout] = useState<WorkspaceLayout>("split");
  const [status, setStatus] = useState<StatusPayload | null>(null);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [tasks, setTasks] = useState<AgentTask[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [image, setImage] = useState<ImageStatusPayload | null>(null);
  const [host, setHost] = useState<HostCapabilities | null>(null);
  const [bootLog, setBootLog] = useState<BootLogPayload | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [draft, setDraft] = useState("");
  const [focused, setFocused] = useState(false);
  const [effects, setEffects] = useState<EffectsTier | "auto">(readEffects);
  const [recommended, setRecommended] = useState<EffectsTier>("reduced");
  const commandRef = useRef<HTMLInputElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const mounted = useRef(false);
  const refreshFlight = useRef<Promise<void> | null>(null);
  const refresh = useCallback((): Promise<void> => {
    if (!native) return Promise.resolve();
    if (refreshFlight.current) return refreshFlight.current;
    const pending = (async () => {
      try {
        const [s, evts, img, h, ts] = await Promise.all([api.getStatus(), api.listEvents(), api.getImageStatus(), api.getHostCapabilities(), api.listTasks()]);
        if (!mounted.current) return;
        setStatus(s); setEvents(evts); setImage(img); setHost(h); setTasks(ts); setConnected(true);
      } catch (e) { if (mounted.current) { setConnected(false); setError(String(e)); } }
    })().finally(() => { refreshFlight.current = null; });
    refreshFlight.current = pending;
    return pending;
  }, [native]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    let disposed = false;
    const cleanups: (() => void)[] = [];
    if (native) {
      void api.suggestedEffects().then((result) => { if (!disposed && isEffectsTier(result.tier)) setRecommended(result.tier); }).catch(() => undefined);
      for (const event of ["pegoles://event", "pegoles://image-progress"]) {
        void listen(event, () => void refresh()).then((stop) => { if (disposed) stop(); else cleanups.push(stop); }).catch((e: unknown) => { if (!disposed) setError(`Live updates unavailable: ${String(e)}`); });
      }
    }
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2500);
    return () => { disposed = true; mounted.current = false; cleanups.forEach((stop) => stop()); window.clearInterval(timer); };
  }, [native, refresh]);

  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault(); setPage("Home"); setSelectedId(null);
        window.setTimeout(() => commandRef.current?.focus(), 0);
      }
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, []);

  const navigate = (next: Page) => { setPage(next); setSelectedId(null); window.setTimeout(() => headingRef.current?.focus(), 0); };
  const run = useCallback(async (action: () => Promise<unknown>) => {
    if (busyRef.current || !native) return;
    busyRef.current = true; setBusy(true); setError(null);
    try { await action(); await refresh(); }
    catch (e) { setError(String(e)); }
    finally { busyRef.current = false; setBusy(false); }
  }, [native, refresh]);
  const selected = tasks.find((task) => task.id === selectedId);
  const tier = effects === "auto" ? recommended : effects;
  const taskPresence = usePresence({ online: connected, task: selected?.status ?? null, viewport: status?.viewport_state ?? null, inputFocused: focused });
  const presence = !connected ? "offline" : error ? "error" : status?.control_owner === "user" ? "waitingForUser" : status?.viewport_state === "agent_active" ? "acting" : taskPresence;
  // Pegoles visually follows its work: drift toward the computer while it
  // acts there, toward the command surface while listening, settle otherwise.
  const lookAt = useMemo(() => {
    if (presence === "acting") return { x: 120, y: -20 };
    if (presence === "listening") return { x: 0, y: 160 };
    if (presence === "waitingForUser") return { x: -80, y: 60 };
    return null;
  }, [presence]);
  const needsImage = status?.backend === "real" && !status.computer_created && image?.status !== "ready";
  const submit = (title: string) => void run(async () => {
    const task = await api.createTask(title);
    setTasks((previous) => [...previous.filter((item) => item.id !== task.id), task]);
    setSelectedId(task.id); setDraft(""); setLayout("split");
  });
  const pending = tasks.filter((task) => ["pending", "running", "waiting_for_approval"].includes(task.status));
  const recent = tasks.filter((task) => !pending.includes(task));
  const openTask = (task: AgentTask) => { setSelectedId(task.id); setLayout("split"); };
  const taskRows = (items: AgentTask[], empty: string) => items.length ? <ul className="task-list">{[...items].reverse().map((task) => <li key={task.id}><button className="task-row" onClick={() => openTask(task)}><TaskIcon size={18} /><span><strong>{task.title}</strong><small>{taskLabel(task)}</small></span><time dateTime={task.updated_at}>{new Date(task.updated_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</time><span aria-hidden="true">›</span></button></li>)}</ul> : <p className="empty-copy">{empty}</p>;
  const computerActions = status && connected ? needsImage ? <div className="computer-actions">{image?.preparing ? <div className="image-progress"><span>{image.stage?.replaceAll("_", " ") ?? "Preparing computer"}</span><ProgressLine label="Computer image preparation" value={image.total > 0 ? image.downloaded / image.total : null} showValue={image.total > 0} /></div> : <GlassButton size="sm" disabled={busy} onClick={() => void run(api.prepareImage)}>Prepare Computer</GlassButton>}</div> : <ComputerControls created={status.computer_created} state={status.computer_state} busy={busy} error={error} onCreate={() => void run(api.createComputer)} onStart={() => void run(api.startComputer)} onPause={() => void run(api.pauseComputer)} onResume={() => void run(api.resumeComputer)} onStop={() => void run(api.stopComputer)} /> : null;
  const userControlling = status?.control_owner === "user";
  useEffect(() => {
    actionCursorSource.setDisplaySize(
      status?.display_config
        ? { w: status.display_config.width_px, h: status.display_config.height_px }
        : null,
    );
  }, [status?.display_config]);
  const computer = <div className="computer-content" data-control={userControlling ? "user" : "agent"}>
    {status && connected ? <div className="computer-viewport-wrap"><NativeComputer enabled={native && status.backend === "real" && status.display_available && (status.computer_state === "running" || status.computer_state === "paused")} onError={setError} state={status.viewport_state} display={status.display_config ?? undefined} subtitle={status.spec_os} errorMessage={status.display_error ?? status.display_setup_error ?? image?.error ?? undefined} offActions={computerActions} headerActions={status.viewport_state !== "off" ? computerActions : undefined} onTakeControl={status.display_attached && !busy ? () => void run(api.takeControl) : undefined} onReturnControl={status.control_owner === "user" ? () => void run(api.returnControl) : undefined} placeholder={<div className="display-empty"><MonitorIcon size={30} /><p>{status.display_available ? "Connecting the computer display" : "No graphical display is available"}</p><small>Only the computer’s actual display appears here.</small></div>} /><AgentCursorOverlay source={actionCursorSource} /></div> : <div className="disconnected-computer"><MonitorIcon size={36} /><h2>Computer unavailable</h2><p>{native ? "Waiting for a connection to Pegoles Core." : "Open the desktop app to connect to Pegoles Computer."}</p></div>}
    {status && <div className="computer-meta"><span>{status.spec_vcpus} CPU · {status.spec_ram_mb / 1024} GB RAM · {status.spec_arch}</span><span>Control: {status.control_owner}</span>{status.backend === "mock" && <span>Backend simulation</span>}</div>}
    {status?.computer_created && status.backend === "real" && <details className="diagnostics" onToggle={(e) => { if (e.currentTarget.open) void api.readBootLog().then(setBootLog).catch((e: unknown) => setError(String(e))); }}><summary>Computer diagnostics</summary>{bootLog?.available ? <pre>{bootLog.tail.join("\n")}</pre> : <p>No boot log available.</p>}</details>}
  </div>;

  const selectedEvents = selected ? events.filter((event) => "task_id" in event && event.task_id === selected.id) : [];
  const selectedDetail = selectedEvents.length ? describeEvent(selectedEvents[selectedEvents.length - 1] as AgentEvent) : undefined;
  const inFocus = selected != null && layout === "focus";

  return <FluxGlassRoot tier={tier} busy={busy || image?.preparing}>
    <div className="app-shell" data-focus={inFocus ? "true" : undefined} data-control={userControlling ? "user" : undefined}>
      <a className="skip-link" href="#main-content">Skip to content</a>
      <GlassSurface as="aside" className="app-rail" material="regular" auditLabel="Navigation rail">
        <button className="brand-button" aria-label="Pegoles Home" onClick={() => navigate("Home")}><PegolesMark size={32} state={presence} lookAt={lookAt} decorative /></button>
        <nav aria-label="Main navigation">{pages.map((item) => <button key={item} className="rail-link" aria-current={page === item && !selected ? "page" : undefined} onClick={() => navigate(item)} title={item}><NavIcon page={item} /><span>{item}</span></button>)}</nav>
        <span className="rail-version">0.1</span>
      </GlassSurface>
      <main id="main-content" className="app-main" tabIndex={-1}>
        <header className="app-topbar"><div className="breadcrumb"><span>Pegoles</span><span aria-hidden="true">/</span><span>{selected ? "Workspace" : page}</span></div><StatusIndicator label={connected ? "Core connected" : native ? "Core disconnected" : "Desktop preview"} tone={connected ? "success" : "neutral"} size="sm" /></header>
        {error && <div className="app-error" role="alert"><span>{error}</span><GlassButton size="sm" variant="quiet" onClick={() => { setError(null); void refresh(); }}>Retry connection</GlassButton><GlassButton size="sm" variant="quiet" onClick={() => setError(null)}>Dismiss</GlassButton></div>}
        {image?.error && <p className="app-error" role="alert">{image.error}</p>}
        <LayoutGroup id="pegoles-workspace">
        {selected ? <TaskWorkspace task={selected} layout={layout} onLayoutChange={setLayout} onBack={() => setSelectedId(null)} headingRef={headingRef} status={status} connected={connected} events={events} taskEvents={selectedEvents} computer={computer} taskDetail={selectedDetail} elapsed={elapsedSince(selected.created_at)} />
        : page === "Home" ? <div className="home-content">
          <section className="home-hero"><PegolesMark size={64} state={presence} lookAt={lookAt} decorative /><h1 ref={headingRef} tabIndex={-1}>What should I do?</h1><div className="home-command" onFocus={() => setFocused(true)} onBlur={() => setFocused(false)}><CommandBar value={draft} onValueChange={setDraft} onSubmit={submit} inputRef={commandRef} placeholder="Give Pegoles a task…" label="New task" disabled={!connected} isBusy={busy} leading={<PegolesMark size={24} state={focused ? "listening" : "idle"} decorative />} shortcutHint="⌘ / Ctrl K" /></div><p className="command-note">{connected ? "Tasks are saved locally. Autonomous execution is not available yet." : "Open the desktop app to create tasks and use your computer."}</p></section>
          <section className="home-section"><div className="section-heading"><h2>Active work</h2><span>{pending.length ? `${pending.length} tasks` : "A clear workspace"}</span></div>{taskRows(pending, "No active tasks. Start something above and Pegoles will work here.")}</section>
          <section className="home-section"><div className="section-heading"><h2>Recent tasks</h2></div>{taskRows(recent, "Your completed tasks will collect here.")}</section>
          <button className="computer-peek" onClick={() => navigate("Computer")}><MonitorIcon size={20} /><span><strong>Pegoles Computer</strong><small>{connected && status ? VIEWPORT_STATE_SPECS[status.viewport_state].label : "Available in the desktop app"}</small></span><span>Open <span aria-hidden="true">↗</span></span></button>
        </div> : page === "Computer" ? <section className="page-content"><div className="page-heading"><h1 ref={headingRef} tabIndex={-1}>Your computer, another space.</h1><p>A dedicated computer for Pegoles. Always clear who’s in control.</p></div>{computer}</section> : page === "Activity" ? <section className="page-content narrow"><div className="page-heading"><h1 ref={headingRef} tabIndex={-1}>Activity</h1><p>What happened, as reported by Pegoles.</p></div><ActivityTimeline events={events} /></section> : page === "Agents" ? <section className="page-content narrow"><div className="page-heading"><h1 ref={headingRef} tabIndex={-1}>Agents</h1><p>A quiet space until there’s work to do.</p></div><div className="agent-profile"><PegolesMark size={56} state={presence} lookAt={lookAt} decorative /><div><h2>Pegoles</h2><p>Local agent</p></div><StatusIndicator label="Execution unavailable" tone="neutral" /></div><p className="empty-copy">Autonomous agents are not available in this version. Tasks can be saved from Home and remain pending.</p><GlassButton variant="secondary" onClick={() => navigate("Home")}>Back to Home</GlassButton></section> : <section className="page-content narrow"><div className="page-heading"><h1 ref={headingRef} tabIndex={-1}>Settings</h1><p>Make this space feel right for your computer.</p></div><section className="settings-section"><h2>Appearance</h2><label className="setting-row"><span><strong>Visual effects</strong><small>Glass, depth, and presence. Minimal uses no blur.</small></span><select value={effects} onChange={(e) => { const value = e.target.value; if (value === "auto" || isEffectsTier(value)) { setEffects(value); try { localStorage.setItem("pegoles.effects", value); } catch { setError("Effects changed for this session; preferences could not be saved."); } } }}><option value="auto">Auto ({recommended})</option><option value="full">Full</option><option value="reduced">Reduced</option><option value="minimal">Minimal</option></select></label><div className="setting-row"><span><strong>Reduced motion</strong><small>Follows your system accessibility preference.</small></span><WindowIcon size={20} /></div></section><section className="settings-section"><h2>System</h2><div className="setting-row"><span>Local model</span><span>{status?.model?.replaceAll("_", " ") ?? "Unavailable"}</span></div><div className="setting-row"><span>Computer host</span><span>{host ? `${host.platform} · ${host.architecture}` : "Desktop app required"}</span></div>{host?.required_setup.length ? <div className="setup-notes"><h3>Setup required</h3><ul>{host.required_setup.map((step) => <li key={step}>{step}</li>)}</ul></div> : null}</section></section>}
        </LayoutGroup>
        <footer className="app-footer"><span>Your AI. Its own computer.</span><span>Pegoles Flux Glass</span></footer>
      </main>
    </div>
  </FluxGlassRoot>;
}

// Re-exported so tree-shaking keeps the morph pair together in this chunk.
export { CommandMorph, TaskController };

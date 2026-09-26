import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { AnimatePresence, LayoutGroup, m } from "motion/react";
import { FluxGlassRoot, type EffectsTier } from "@pegoles/ui";
import { PresenceGpuShareProvider, PresenceQualityProvider } from "./presence";
import { useCore } from "./state/useCore";
import { actionSteps, globalPresence, modelConnected, taskActivity, type TaskActivity } from "./state/agentState";
import { buildTranscript } from "./state/transcript";
import { computerModel, withCommandError, type ComputerCommand } from "./state/computerModel";
import { taskSections } from "./state/taskSections";
import { eventsForTask } from "./lib/execution";
import { api, type AgentTask } from "./lib/tauri";
import { duration, ease } from "./lib/motion";
import { formatElapsed, modelLabel, shortcutModifier } from "./lib/format";
import { activityPill } from "./lib/taskState";
import { useSystemReducedMotion } from "./lib/useSystemReducedMotion";
import { Sidebar, type Place, type SidebarMode, type TaskGlance } from "./shell/Sidebar";
import { Toolbar, type ToolbarContext } from "./shell/Toolbar";
import { CommandPalette } from "./shell/CommandPalette";
import { useNow } from "./shell/useNow";
import { useAccessibilityDisplay } from "./shell/useAccessibilityDisplay";
import { useMediaQuery } from "./shell/useMediaQuery";
import { useWindowWidth } from "./shell/useWindowWidth";
import { Announcer } from "./shell/Announcer";
import { Toast } from "./shell/Toast";
import { HomeView } from "./home/HomeView";
import { Composer } from "./composer/Composer";
import { ComposerControls, ComposerStrip } from "./composer/ComposerContext";
import { TaskView } from "./task/TaskView";
import { StatusDock } from "./task/StatusDock";
import { COMPUTER_PANEL_ID, ComputerPanel } from "./computer/ComputerPanel";
import { ComputerPeek } from "./computer/ComputerPeek";
import { columns, stepBack, type ComputerLevel } from "./computer/layout";
import { useComputerLevel } from "./computer/useComputerLevel";
import { useScreenSnapshot } from "./computer/useScreenSnapshot";
import type { ManageCommand } from "./computer/ComputerManage";
import { useModelSettings } from "./state/useModelSettings";
import { intelligenceProblem, useIntelligence } from "./state/useIntelligence";
import { localView } from "./state/localModel";
import { ActivityView } from "./pages/ActivityView";
import { SettingsView, type QualityChoice, type SettingsAnchor } from "./pages/SettingsView";

type View = { readonly kind: "home" } | { readonly kind: "task"; readonly id: string } | { readonly kind: "place"; readonly place: Place };

const QUALITY_KEY = "pegoles.quality";
const SIDEBAR_KEY = "pegoles.sidebar";
/** Long enough for the acknowledgement to be felt, short enough to never stall. */
const ACK_MIN_MS = 260;
const ACK_TOTAL_MS = 420;
const ARRIVAL_MS = 900;

function readQuality(): QualityChoice {
  try {
    const stored = localStorage.getItem(QUALITY_KEY) ?? localStorage.getItem("pegoles.effects");
    if (stored === "full" || stored === "reduced" || stored === "auto") return stored;
    return stored === "minimal" ? "reduced" : "auto";
  } catch { return "auto"; }
}

function readSidebarHidden(): boolean {
  try { return localStorage.getItem(SIDEBAR_KEY) === "rail" || localStorage.getItem(SIDEBAR_KEY) === "hidden"; } catch { return false; }
}

/** "2m" while Pegoles works, "4m" of work once it settles: only from Core's own timestamps. */
function elapsedOf(task: AgentTask, activity: TaskActivity, now: number): string | undefined {
  const since = new Date(task.created_at).getTime();
  if (!Number.isFinite(since)) return undefined;
  if (task.status === "running" && activity.live) return formatElapsed(now - since);
  return undefined;
}

const wait = (delay: number) => new Promise<void>((resolve) => { window.setTimeout(resolve, delay); });

export default function App() {
  const core = useCore();
  const { connected, status, tasks, events, run, report } = core;
  const systemReducedMotion = useSystemReducedMotion();
  useAccessibilityDisplay(core.native);
  const narrow = useMediaQuery("(max-width: 1099px)");
  const { width: windowWidth, resizing } = useWindowWidth();

  const [view, setView] = useState<View>({ kind: "home" });
  const [quality, setQuality] = useState<QualityChoice>(readQuality);
  const tier: EffectsTier = quality === "auto" ? core.recommendedTier : quality;
  const animated = !systemReducedMotion && tier !== "minimal";
  const computerLevel = useComputerLevel(animated);
  const { level, mounted, motion } = computerLevel;
  /** The panel was opened by the person (focus moves into it), not by Pegoles. */
  const [panelFocus, setPanelFocus] = useState(false);
  const opener = useRef<HTMLElement | null>(null);
  const [sidebarHidden, setSidebarHidden] = useState(readSidebarHidden);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [focused, setFocused] = useState(false);
  const [ackTask, setAckTask] = useState<string | null>(null);
  const [arriving, setArriving] = useState<string | null>(null);
  const [created, setCreated] = useState<AgentTask | null>(null);
  const [nudge, setNudge] = useState(0);
  const [headingVisible, setHeadingVisible] = useState(true);
  const [anchor, setAnchor] = useState<SettingsAnchor | null>(null);
  const [peekHidden, setPeekHidden] = useState<ReadonlySet<string>>(new Set());
  /** The task the last start was for: its failure is shown on that task only. */
  const [startedFor, setStartedFor] = useState<string | null>(null);
  const modelSettings = useModelSettings(core.native && connected, core.refresh);
  const intelligence = useIntelligence(core.native && connected, core.refresh);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const openWidth = useRef(0);

  // ── Derived state ────────────────────────────────────────────────
  const modelReady = modelConnected(status);
  const provider = status?.provider ?? intelligence.intelligence?.provider ?? "local";
  const local = useMemo(() => localView(intelligence.intelligence), [intelligence.intelligence]);
  const selected = view.kind === "task"
    ? tasks.find((task) => task.id === view.id) ?? (created?.id === view.id ? created : undefined)
    : undefined;
  const selectedEvents = useMemo(() => (selected ? eventsForTask(events, selected.id) : []), [events, selected]);
  const starting = !!selected && core.busy.run && startedFor === selected.id;
  const activity = useMemo(() => selected && taskActivity({
    task: selected, events: selectedEvents, status, connected, acknowledging: ackTask === selected.id, starting,
  }), [selected, selectedEvents, status, connected, ackTask, starting]);
  const transcript = useMemo(() => (selected ? buildTranscript({ task: selected, events: selectedEvents }) : []), [selected, selectedEvents]);

  const computer = withCommandError(computerModel({ connected, native: core.native, status, events }), !!core.errors.computer);
  const presence = globalPresence({
    connected, tasks, status, acknowledging: ackTask === "pending", attentive: focused || draft.trim().length > 0,
    computerTransitioning: computer.transitioning,
  });

  const liveWork = tasks.some((task) => task.status === "running");
  const now = useNow(true, liveWork ? 15_000 : 60_000);
  const sections = useMemo(() => taskSections(tasks, now), [tasks, now]);
  const glances = useMemo(() => {
    const entries: Record<string, TaskGlance | null> = {};
    for (const task of tasks) {
      if (task.status === "running") {
        const live = taskActivity({ task, events: eventsForTask(events, task.id), status, connected });
        entries[task.id] = { text: live.headline, tone: live.mode === "waiting" ? "attention" : "live" };
      } else if (task.status === "waiting_for_approval") entries[task.id] = { text: "Needs your approval", tone: "attention" };
      else entries[task.id] = null;
    }
    return entries;
  }, [tasks, events, status, connected]);
  const waiting = tasks.find((task) => task.status === "waiting_for_approval");

  const pill = selected && activity ? activityPill(activity.mode, selected.status) : null;
  const elapsed = selected && activity ? elapsedOf(selected, activity, now) : undefined;
  const screenUp = computer.running || computer.phase === "paused";
  /** A run is working on the task in view: Stop cancels that task. */
  const selectedRunning = !!selected && (selected.status === "running" || status?.active_task === selected.id);
  const locked = computer.owner === "user";
  const slotEnabled = core.native && status?.backend === "real" && !!status.display_available &&
    (status.computer_state === "running" || status.computer_state === "paused");

  // ── Spatial layout ──────────────────────────────────────────────
  const sidebarMode: SidebarMode = narrow ? (drawerOpen ? "drawer" : "hidden") : sidebarHidden || level === "full" ? "hidden" : "shown";
  const cols = columns(level, windowWidth, sidebarMode === "shown");
  if (level !== null && motion !== "close") openWidth.current = cols.computer;
  const innerOpenWidth = motion === "open" || motion === "close" ? openWidth.current : 0;
  const shellStyle = { "--col-sidebar": `${cols.sidebar}px`, "--col-computer": `${cols.computer}px` } as CSSProperties;

  const peekVisible = view.kind === "task" && !!selected && activity?.mode === "using-computer" && level === null &&
    mounted === null && !peekHidden.has(selected.id);
  const snapshot = useScreenSnapshot({
    enabled: core.native && computer.running && !slotEnabled && (mounted !== null || peekVisible),
    trigger: events.length,
    capture: api.captureScreen,
  });
  const panelSteps = useMemo(
    () => [...(activity ? activity.recent : actionSteps(events).slice(-5))].reverse().slice(0, 5),
    [activity, events],
  );

  // ── Navigation ──────────────────────────────────────────────────
  const focusHeading = useCallback(() => {
    window.setTimeout(() => headingRef.current?.focus({ preventScroll: true }), 0);
  }, []);
  const setLevel = computerLevel.setLevel;
  const leaveFull = useCallback(() => { if (level === "full") setLevel("focus"); }, [level, setLevel]);
  const newTask = useCallback(() => {
    setView({ kind: "home" });
    setDrawerOpen(false);
    if (level === "full" && !locked) setLevel("side");
    window.setTimeout(() => composerRef.current?.focus(), 0);
  }, [level, locked, setLevel]);
  const openTask = useCallback((task: AgentTask) => {
    setView({ kind: "task", id: task.id });
    setHeadingVisible(true);
    setDrawerOpen(false);
    leaveFull();
    focusHeading();
  }, [leaveFull, focusHeading]);
  const openPlace = useCallback((next: Place, section: SettingsAnchor | null = null) => {
    setView({ kind: "place", place: next });
    setAnchor(section);
    setDrawerOpen(false);
    leaveFull();
    if (!section) focusHeading();
  }, [leaveFull, focusHeading]);
  const openComputer = useCallback((next: ComputerLevel = "side") => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setPanelFocus(true);
    setLevel(next);
  }, [setLevel]);
  const closeComputer = useCallback(() => {
    if (locked) return;
    const back = opener.current?.isConnected ? opener.current : document.querySelector<HTMLElement>(".toolbar__computer");
    window.setTimeout(() => back?.focus({ preventScroll: true }), 0);
    // Retract from wherever the edge is right now, even mid-resize.
    const width = document.getElementById(COMPUTER_PANEL_ID)?.getBoundingClientRect().width;
    if (width) openWidth.current = Math.round(width);
    setLevel(null);
  }, [locked, setLevel]);
  const toggleComputer = useCallback(() => {
    setDrawerOpen(false);
    if (level) closeComputer(); else openComputer();
  }, [level, openComputer, closeComputer]);
  const changeLevel = useCallback((next: ComputerLevel) => {
    setPanelFocus(next === "full" || level === "full");
    // Focus and Full only mean something once there is a screen.
    setLevel(next !== "side" && !screenUp ? "side" : next);
  }, [screenUp, level, setLevel]);
  const toggleSidebar = useCallback(() => {
    if (narrow) { setDrawerOpen((value) => !value); return; }
    setSidebarHidden((value) => {
      const next = !value;
      try { localStorage.setItem(SIDEBAR_KEY, next ? "hidden" : "shown"); } catch { /* applies to this session */ }
      return next;
    });
  }, [narrow]);

  // Focus and Full need a screen: when the machine goes away, step back to Side.
  useEffect(() => {
    if (!screenUp && (level === "focus" || level === "full")) setLevel("side");
  }, [screenUp, level, setLevel]);

  // ── Runs ────────────────────────────────────────────────────────
  const startTask = useCallback((taskId: string) => {
    setStartedFor(taskId);
    void run("run", () => api.runTask(taskId));
  }, [run]);
  /** Stop a task's run; with none, interrupt whatever input is in flight (which also stops any run). */
  const stopTask = useCallback((taskId: string | null) => {
    void run("general", () => (taskId ? api.cancelTask(taskId) : api.cancelAgentInput()));
  }, [run]);

  // ── Hand-off: Home → task, as one continuous scene ─────────────
  const [handing, setHanding] = useState(false);
  const submit = async (title: string) => {
    setHanding(true);
    setAckTask("pending");
    const [task] = await Promise.all([run("task", () => api.createTask(title)), wait(ACK_MIN_MS)]);
    setHanding(false);
    if (!task) {
      setAckTask(null);
      window.setTimeout(() => composerRef.current?.focus(), 0);
      return;
    }
    setCreated(task);
    setDraft("");
    setFocused(false);
    setAckTask(task.id);
    setArriving(task.id);
    setHeadingVisible(true);
    setView({ kind: "task", id: task.id });
    focusHeading();
    // With a model connected the job starts at once; otherwise it waits, and says why.
    if (modelReady && !status?.active_task) startTask(task.id);
  };
  useEffect(() => {
    if (!ackTask || ackTask === "pending") return;
    const timer = window.setTimeout(() => setAckTask(null), ACK_TOTAL_MS - ACK_MIN_MS);
    return () => window.clearTimeout(timer);
  }, [ackTask]);
  useEffect(() => {
    if (!arriving) return;
    const timer = window.setTimeout(() => setArriving(null), ARRIVAL_MS);
    return () => window.clearTimeout(timer);
  }, [arriving]);

  // ── Computer commands ──────────────────────────────────────────
  // Starting over is refused while a task runs; a failure is said in a toast, not as "can't start".
  const manageComputer = useCallback((command: ManageCommand) => {
    void run("general", command === "reset" ? api.resetComputer : api.destroyComputer);
  }, [run]);
  const runComputer = useCallback((command: ComputerCommand) => {
    const actions: Record<Exclude<ComputerCommand, ManageCommand>, () => Promise<unknown>> = {
      start: async () => {
        if (!status?.computer_created) await api.createComputer();
        return api.startComputer();
      },
      pause: api.pauseComputer,
      resume: api.resumeComputer,
      stop: api.stopComputer,
      take: api.takeControl,
      return: api.returnControl,
    };
    if (command === "reset" || command === "remove") { manageComputer(command); return; }
    void run("computer", actions[command]);
  }, [run, status?.computer_created, manageComputer]);
  const interrupt = useCallback(() => { stopTask(status?.active_task ?? null); }, [stopTask, status?.active_task]);

  // ── Pegoles Local ──────────────────────────────────────────────
  const { install: installLocal, repair: repairLocal, cancelInstall: cancelLocal } = intelligence;
  const setUpLocal = useCallback(() => {
    if (!local.model) return;
    void (local.stage === "damaged" ? repairLocal(local.model.id) : installLocal(local.model.id));
  }, [local.model, local.stage, installLocal, repairLocal]);
  const cancelLocalSetup = useCallback(() => { void cancelLocal(); }, [cancelLocal]);
  const reportSlot = useCallback((error: unknown) => report("computer", error), [report]);

  // ── Keyboard ───────────────────────────────────────────────────
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      if (mod && key === "k") { event.preventDefault(); setPaletteOpen((open) => !open); return; }
      if (mod && key === "n") { event.preventDefault(); setPaletteOpen(false); newTask(); return; }
      if (mod && event.key === "\\") { event.preventDefault(); toggleSidebar(); return; }
      if (mod && key === "j") { event.preventDefault(); setPaletteOpen(false); toggleComputer(); return; }
      if (mod && event.key === "." && (status?.agent_busy || status?.active_task)) { event.preventDefault(); interrupt(); return; }
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (paletteOpen) { setPaletteOpen(false); return; }
      if (narrow && drawerOpen) { setDrawerOpen(false); return; }
      // While a person controls the computer, Escape belongs to the guest.
      if (locked || !level) return;
      if (level === "side" && document.activeElement instanceof HTMLTextAreaElement) return;
      const back = stepBack(level);
      if (back) setLevel(back); else closeComputer();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newTask, toggleSidebar, toggleComputer, interrupt, status?.agent_busy, status?.active_task, narrow, drawerOpen, paletteOpen, level, locked, closeComputer, setLevel]);

  const chooseQuality = (value: QualityChoice) => {
    setQuality(value);
    try { localStorage.setItem(QUALITY_KEY, value); } catch { /* the choice still applies to this session */ }
  };

  const modifier = shortcutModifier();
  // The composer names what will think: Pegoles Local, or the cloud model when that was chosen.
  const modelName = provider === "anthropic" ? (modelSettings.settings ? modelLabel(modelSettings.settings.model) : undefined) : "Pegoles Local";
  const setupLabel = provider === "anthropic" ? undefined : local.stage === "preparing" ? "Setting up Pegoles Local…" : "Set up Pegoles Local";

  // ── The work column's views ─────────────────────────────────────
  let content: ReactNode;
  if (view.kind === "task" && selected && activity && pill) {
    content = (
      <TaskView
        key={selected.id}
        task={selected}
        items={transcript}
        activity={activity}
        state={pill}
        elapsed={elapsed}
        animated={animated}
        arriving={arriving === selected.id}
        headingRef={headingRef}
        onHeadingVisible={setHeadingVisible}
        onOpenComputer={() => openComputer()}
        onModelSettings={() => openPlace("settings", "intelligence")}
        onStart={() => startTask(selected.id)}
        starting={starting}
        startError={startedFor === selected.id ? core.errors.run : null}
        local={activity.waitingFor === "local-model" ? {
          view: local, busy: intelligence.pending !== null, problem: intelligenceProblem(intelligence.error, ["install", "cancel"]),
          onSetUp: setUpLocal, onCancel: cancelLocalSetup,
        } : undefined}
      />
    );
  } else if (view.kind === "place" && view.place === "activity") {
    content = <ActivityView events={events} tasks={tasks} headingRef={headingRef} onOpenTask={openTask} />;
  } else if (view.kind === "place" && view.place === "settings") {
    content = (
      <SettingsView
        headingRef={headingRef} connected={connected} native={core.native} status={status} host={core.host} computer={computer}
        intelligence={intelligence} model={modelSettings} quality={quality} resolvedQuality={tier === "full" ? "full" : "reduced"} onQuality={chooseQuality}
        systemReducedMotion={systemReducedMotion} eventCount={events.length} anchor={anchor}
      />
    );
  } else {
    content = (
      <HomeView
        presence={presence}
        headingRef={headingRef}
        attentive={focused || draft.trim().length > 0}
        nudge={nudge}
        firstRun={tasks.length === 0}
        onStarter={(prompt) => { setDraft(prompt); window.setTimeout(() => composerRef.current?.focus(), 0); }}
        onPressPresence={() => composerRef.current?.focus()}
        animated={animated}
      />
    );
  }
  const viewKey = view.kind === "task" ? `task-${view.id}` : view.kind === "place" ? view.place : "home";
  const bar = view.kind === "home" ? "composer" : view.kind === "task" && activity && pill ? "dock" : null;

  const composer = (
    <Composer
      value={draft}
      onChange={setDraft}
      onSubmit={(title) => void submit(title)}
      onFocusChange={setFocused}
      onKeystroke={() => setNudge((value) => value + 1)}
      inputRef={composerRef}
      placeholder={connected ? "Describe a job for Pegoles…" : "Waiting for Pegoles Core…"}
      disabled={!connected}
      busy={core.busy.task || handing}
      problem={core.errors.task ? `${core.errors.task.title} ${core.errors.task.hint ?? ""}`.trim() : null}
      strip={connected && <ComposerStrip computer={computer} computerOpen={level !== null} onComputer={toggleComputer} />}
      controls={connected && (
        <ComposerControls modelReady={modelReady} modelName={modelName} setupLabel={setupLabel}
          onSafety={() => openPlace("settings", "security")} onModel={() => openPlace("settings", "intelligence")} />
      )}
    />
  );

  return (
    <FluxGlassRoot tier={tier} busy={core.busy.computer || !!status?.agent_busy || !!status?.active_task || liveWork}>
      <PresenceQualityProvider quality={quality}>
        <PresenceGpuShareProvider shared={level !== null}>
        <LayoutGroup id="pegoles-shell">
          <div
            className="shell"
            style={shellStyle}
            data-sidebar={sidebarMode}
            data-computer={level ?? "closed"}
            data-computer-phase={computer.phase}
            data-view={view.kind}
            data-resizing={resizing || undefined}
          >
            <a className="skip-link" href="#main-content">Skip to content</a>
            <Sidebar
              sections={sections}
              glances={glances}
              selectedTaskId={selected?.id ?? null}
              place={view.kind === "place" ? view.place : null}
              presence={presence}
              computer={computer}
              computerOpen={level !== null}
              connected={connected}
              mode={sidebarMode}
              modifier={modifier}
              onOpenTask={openTask}
              onNewTask={newTask}
              onSearch={() => setPaletteOpen(true)}
              onPlace={(next) => openPlace(next)}
              onComputer={toggleComputer}
              onToggle={toggleSidebar}
            />
            {sidebarMode === "drawer" && <button type="button" className="scrim" aria-label="Close sidebar" onClick={() => setDrawerOpen(false)} />}

            <main id="main-content" className="work" tabIndex={-1} aria-hidden={level === "full" || undefined}
              {...(sidebarMode === "drawer" || level === "full" ? { inert: "" } : {})}>
              <Toolbar
                sidebarHidden={sidebarMode !== "shown"}
                context={view.kind === "task" && selected && pill && !headingVisible ? { title: selected.title, tone: pill.tone, live: pill.live } satisfies ToolbarContext : null}
                computer={computer}
                computerOpen={level !== null}
                showComputerToggle={level !== "full"}
                modifier={modifier}
                onToggleSidebar={toggleSidebar}
                onNewTask={newTask}
                onToggleComputer={toggleComputer}
              />
              <div className="work__views">
                <AnimatePresence mode="popLayout" initial={false}>
                  <m.div
                    key={viewKey}
                    className="view"
                    data-view={view.kind}
                    initial={view.kind === "task" && arriving ? { opacity: 1 } : animated ? { opacity: 0, y: 6 } : { opacity: 0 }}
                    animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}
                    exit={{ opacity: 0, transition: { duration: duration.micro, ease: ease.exit } }}
                  >
                    {content}
                  </m.div>
                </AnimatePresence>
              </div>
              <div className="workbar" data-empty={bar ? undefined : true}>
                <AnimatePresence mode="popLayout" initial={false}>
                  {bar === "composer" && waiting && (
                    <m.div key="needs-you" className="banner" data-tone="attention" role="status"
                      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}
                      exit={{ opacity: 0, transition: { duration: duration.micro, ease: ease.exit } }}>
                      <span className="banner__dot" aria-hidden="true" />
                      <span className="banner__text">
                        <span className="banner__title">Pegoles needs your approval</span>
                        <span className="banner__body">{waiting.title}</span>
                      </span>
                      <button type="button" className="btn btn--line btn--small" onClick={() => openTask(waiting)}>Open task</button>
                    </m.div>
                  )}
                  {bar === "composer" && (
                    <m.div key="composer" className="workbar__composer"
                      initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out } }}
                      exit={{ opacity: 0, y: 6, scale: 0.985, transition: { duration: duration.micro, ease: ease.exit } }}>
                      {composer}
                    </m.div>
                  )}
                  {bar === "dock" && selected && activity && pill && (
                    <m.div key={`dock-${selected.id}`} className="workbar__dock"
                      exit={{ opacity: 0, y: 6, transition: { duration: duration.micro, ease: ease.exit } }}>
                      <StatusDock
                        activity={activity}
                        state={pill}
                        elapsed={elapsed}
                        arriving={arriving === selected.id}
                        interruptible={activity.live && (selectedRunning || (!!status?.agent_busy && !status?.active_task))}
                        interrupting={core.busy.general}
                        computerOpen={level !== null}
                        modifier={modifier}
                        onInterrupt={() => stopTask(selectedRunning ? selected.id : null)}
                        onWatch={() => openComputer()}
                        onNewTask={newTask}
                        settingUp={local.stage === "preparing"}
                      />
                    </m.div>
                  )}
                </AnimatePresence>
              </div>
              <AnimatePresence>
                {peekVisible && selected && activity && (
                  <ComputerPeek
                    key="peek"
                    model={computer}
                    snapshot={snapshot}
                    caption={activity.detail ? `${activity.headline} · ${activity.detail}` : activity.headline}
                    onOpen={() => openComputer()}
                    onDismiss={() => setPeekHidden((previous) => new Set([...previous, selected.id]))}
                  />
                )}
              </AnimatePresence>
            </main>

            {mounted && (
              <ComputerPanel
                model={computer}
                level={mounted}
                status={status}
                slotEnabled={slotEnabled}
                snapshot={core.native ? snapshot : null}
                steps={panelSteps}
                busy={core.busy.computer}
                managing={core.busy.general}
                error={core.errors.computer}
                moving={motion !== null}
                obscured={sidebarMode === "drawer" || paletteOpen || level === null || (mounted !== "side" && !!core.errors.general)}
                focusOnOpen={panelFocus}
                onCommand={runComputer}
                onManage={manageComputer}
                onLevel={changeLevel}
                onClose={closeComputer}
                onSlotError={reportSlot}
                onDismissError={() => core.dismiss("computer")}
                loadBootLog={api.readBootLog}
                style={innerOpenWidth ? ({ "--open-w": `${innerOpenWidth}px` } as CSSProperties) : undefined}
              />
            )}
            <AnimatePresence>
              {paletteOpen && (
                <CommandPalette
                  key="palette"
                  tasks={tasks}
                  computerWord={computer.chip}
                  computerOpen={level !== null}
                  modifier={modifier}
                  onClose={() => setPaletteOpen(false)}
                  onNewTask={newTask}
                  onOpenTask={openTask}
                  onComputer={toggleComputer}
                  onActivity={() => openPlace("activity")}
                  onSettings={() => openPlace("settings")}
                  onToggleSidebar={toggleSidebar}
                />
              )}
            </AnimatePresence>
            <Toast error={core.errors.general} onDismiss={() => core.dismiss("general")} />
            <Announcer tasks={tasks} computerPhase={computer.phase} />
          </div>
        </LayoutGroup>
        </PresenceGpuShareProvider>
      </PresenceQualityProvider>
    </FluxGlassRoot>
  );
}

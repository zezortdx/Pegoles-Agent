/**
 * DEV ONLY — never shipped (main.tsx gates it on import.meta.env.DEV and the
 * production bundle test checks for the marker below).
 *
 * Installs a simulated Tauri bridge so the REAL App can be rendered in every
 * product state for visual QA: `#/dev/shell/<scenario>`. Nothing here is
 * reachable from production code paths.
 */
import {
  MODEL_INSTALL_EVENT, type AgentEvent, type AgentMessageKind, type AgentTask, type Intelligence, type LocalModelInfo, type ModelInstallStatus,
  type ModelSettings, type Provider, type StatusPayload,
} from "../lib/tauri";
import { installActive } from "../state/localModel";

export const SHELL_LAB_MARKER = "__PEGOLES_SHELL_LAB__";

/** Pegoles Local's states (append `/settings` to land on Settings, e.g. `#/dev/shell/local-downloading/settings`). */
type IntelligenceScenario =
  | "local-setup" | "local-downloading" | "local-verifying" | "local-ready" | "local-running" | "local-failed"
  | "local-paused" | "local-damaged" | "local-unsupported" | "cloud";

type Scenario =
  | "home" | "pending" | "ready" | "thinking" | "running" | "files" | "computer" | "user" | "approval"
  | "done" | "cancelled" | "failure" | "long" | "booting" | "offline-setup" | "paused" | IntelligenceScenario;

const now = Date.now();
const iso = (secondsAgo: number) => new Date(now - secondsAgo * 1000).toISOString();

const baseStatus: StatusPayload = {
  core: "running", model: "not_configured", provider: "local", backend: "real", computer_created: false, computer_state: null, computer_id: null,
  image_status: "ready", spec_os: "Debian 13", spec_arch: "arm64", spec_vcpus: 2, spec_ram_mb: 1536, guest_state: "unavailable",
  guest_ready_ms: null, viewport_state: "off", viewport_issue: null, display_available: false, display_attached: false,
  display_config: { width_px: 1440, height_px: 900 }, display_error: null, display_setup_error: null, control_owner: "none",
  input_available: false, agent_busy: false, active_task: null,
};
const running: Partial<StatusPayload> = {
  model: "configured", computer_created: true, computer_state: "running", computer_id: "vm-1", guest_state: "ready", guest_ready_ms: 4200,
  viewport_state: "ready", input_available: true,
};

const task = (id: string, title: string, status: AgentTask["status"], ago: number): AgentTask =>
  ({ id, title, status, created_at: iso(ago), updated_at: iso(Math.max(0, ago - 40)) });

let actionSeq = 0;
function action(taskId: string, type: string, extra: Record<string, unknown>, ago: number, outcome: "executed" | "running" | "denied" | "needs_approval" = "executed"): AgentEvent[] {
  actionSeq += 1;
  const request = { action_id: `act-${actionSeq}`, task_id: taskId, computer_id: "vm-1", action: { type, ...extra }, requested_at: iso(ago) };
  const started = { type: "action_started", action_id: request.action_id, request, at: iso(ago) } as AgentEvent;
  if (outcome === "running") return [started];
  if (outcome === "denied") return [started, { type: "action_denied", request, reason: "host path access denied" } as AgentEvent];
  return [started, {
    type: "action_completed", request,
    result: { action_id: request.action_id, success: outcome === "executed", outcome, message: "", duration_ms: 40 + (actionSeq % 7) * 23 },
  } as AgentEvent];
}

function note(taskId: string, kind: AgentMessageKind, text: string, ago: number): AgentEvent {
  return { type: "agent_message", task_id: taskId, kind, text, at: iso(ago) };
}

const history: AgentTask[] = [
  task("h1", "Rename the vacation photos by date", "completed", 60 * 60 * 26),
  task("h2", "Summarize the Q3 board deck", "completed", 60 * 60 * 3),
];

interface World { status: StatusPayload; tasks: AgentTask[]; events: AgentEvent[]; failComputer?: string }

function world(scenario: Scenario): World {
  const current = (status: AgentTask["status"], title = "Find duplicate files in Downloads and clean them up") => task("t1", title, status, 95);
  switch (scenario) {
    case "home": return { status: baseStatus, tasks: [], events: [] };
    case "pending": return { status: baseStatus, tasks: [...history, current("pending")], events: [{ type: "task_created", task_id: "t1", title: "Find duplicate files in Downloads and clean them up", at: iso(95) }] };
    case "ready": return { status: { ...baseStatus, model: "configured" }, tasks: [...history, current("pending")], events: [{ type: "task_created", task_id: "t1", title: "Find duplicate files in Downloads and clean them up", at: iso(95) }] };
    case "running": return {
      status: { ...baseStatus, ...running, viewport_state: "agent_active", control_owner: "agent", active_task: "t1" },
      tasks: [...history, current("running", "Compare the pricing of the three most popular note-taking apps")],
      events: [
        note("t1", "progress", "I’ll start by opening Firefox and searching for each app’s pricing page.", 88),
        ...action("t1", "observe_screen", {}, 86), ...action("t1", "double_click", { x: 0.08, y: 0.12 }, 84), ...action("t1", "observe_screen", {}, 80),
        note("t1", "progress", "Firefox is open. Searching for Notion’s pricing first, then Obsidian and Evernote.\nI’ll note the monthly price of each paid plan.", 60),
        ...action("t1", "click", { x: 0.5, y: 0.08 }, 58), ...action("t1", "type_text", { text: "", sensitive: false }, 56), ...action("t1", "key_press", { key: "Return" }, 54),
        ...action("t1", "observe_screen", {}, 50), ...action("t1", "scroll", { x: 0.5, y: 0.5, delta_x: 0, delta_y: 5 }, 46),
        note("t1", "progress", "Notion Plus is $10 per seat a month. Opening Obsidian’s pricing next.", 20),
        ...action("t1", "click", { x: 0.5, y: 0.08 }, 2, "running"),
      ],
    };
    case "thinking": return { status: { ...baseStatus, ...running, model: "configured" }, tasks: [...history, current("running")], events: [] };
    case "files": return {
      status: { ...baseStatus, ...running, agent_busy: true },
      tasks: [...history, current("running")],
      events: [...action("t1", "read_file", { path: "/home/pegoles/workspace/Downloads/index.txt" }, 70), ...action("t1", "list_directory", { path: "/home/pegoles/workspace/Downloads" }, 60), ...action("t1", "write_file", { path: "/home/pegoles/workspace/duplicates.md", content: "…" }, 40), ...action("t1", "read_file", { path: "/home/pegoles/workspace/Downloads/IMG_2041.jpg" }, 2, "running")],
    };
    case "computer": return {
      status: { ...baseStatus, ...running, viewport_state: "agent_active", control_owner: "agent", agent_busy: true },
      tasks: [...history, current("running")],
      events: [...action("t1", "screenshot", {}, 50), ...action("t1", "click", { x: 0.2, y: 0.4 }, 40), ...action("t1", "type_text", { text: "", sensitive: false }, 30), ...action("t1", "click", { x: 0.6, y: 0.3 }, 1, "running")],
    };
    case "user": return {
      status: { ...baseStatus, ...running, viewport_state: "user_controlled", control_owner: "user", display_attached: true },
      tasks: [...history, current("running", "Sign in to the supplier portal and download the invoices")],
      events: [...action("t1", "open_url", { url: "https://portal.example.com/login" }, 50), ...action("t1", "screenshot", {}, 45)],
    };
    case "approval": return {
      status: { ...baseStatus, ...running },
      tasks: [...history, current("waiting_for_approval")],
      events: [...action("t1", "list_directory", { path: "/home/pegoles/workspace/Downloads" }, 60), ...action("t1", "open_url", { url: "https://www.dropbox.com/home" }, 30, "needs_approval"),
        { type: "approval_requested", task_id: "t1", reason: "Open dropbox.com to compare the duplicates with your cloud copies", at: iso(29) }],
    };
    case "done": return {
      status: { ...baseStatus, ...running },
      tasks: [...history, { ...current("completed"), updated_at: iso(5) }],
      events: [...action("t1", "list_directory", { path: "/home/pegoles/workspace/Downloads" }, 80), ...action("t1", "read_file", { path: "/home/pegoles/workspace/Downloads/index.txt" }, 70),
        ...action("t1", "write_file", { path: "/home/pegoles/workspace/Downloads/duplicates-report.md", content: "…" }, 20), ...action("t1", "shell", { command: "ls" }, 10),
        note("t1", "summary", [
          "I found 14 duplicate files in Downloads (1.2 GB in total) and listed them in duplicates-report.md, grouped by original.",
          "", "Nothing was deleted: 3 of the pairs differ slightly (different export dates), so please check those before removing anything.",
          "", "Largest duplicates:", "• Keynote export (2).mov — 640 MB", "• IMG_2041 copy.jpg — 12 MB", "• Invoice-2026-08 (1).pdf — 1.1 MB",
          "", "If you want, start a new task to move the rest to the Trash.",
        ].join("\n"), 6)],
    };
    case "cancelled": return {
      status: { ...baseStatus, ...running, model: "configured" },
      tasks: [...history, { ...current("cancelled"), updated_at: iso(8) }],
      events: [note("t1", "progress", "Opening the Files app to look through Downloads.", 60), ...action("t1", "observe_screen", {}, 58), ...action("t1", "double_click", { x: 0.1, y: 0.2 }, 50),
        note("t1", "error", "Stopped by the user.", 9)],
    };
    case "failure": return { status: baseStatus, tasks: history, events: [], failComputer: "computer error: backend error: pegoles-vm-host binary not found (build the native helper or set PEGOLES_VM_HOST)" };
    case "long": {
      const events: AgentEvent[] = [];
      for (let i = 0; i < 18; i += 1) events.push(...action("t1", i % 3 === 0 ? "screenshot" : i % 3 === 1 ? "click" : "scroll", { x: 0.5, y: 0.5, delta_x: 0, delta_y: 3 }, 300 - i * 12));
      events.push(...action("t1", "write_file", { path: "/home/pegoles/workspace/research/competitors-2026.md", content: "…" }, 70));
      events.push(...action("t1", "read_file", { path: "/home/pegoles/workspace/research/notes/pricing-comparison-with-a-very-long-file-name-for-layout.md" }, 50));
      events.push(...action("t1", "open_url", { url: "https://www.example.com/pricing" }, 2, "running"));
      return {
        status: { ...baseStatus, ...running, agent_busy: true },
        tasks: [...history, ...Array.from({ length: 7 }, (_, i) => task(`o${i}`, `Older task number ${i + 1} with a reasonably long title`, "completed", 60 * 60 * (30 + i))),
          current("running", "Research the top five competitors in agent desktop software, compare their pricing and write a short report with sources")],
        events,
      };
    }
    case "booting": return {
      status: { ...baseStatus, computer_created: true, computer_state: "starting", viewport_state: "guest_connecting", guest_state: "connecting" },
      tasks: history, events: [],
    };
    case "offline-setup": return { status: { ...baseStatus, image_status: "missing" }, tasks: [], events: [] };
    case "paused": return {
      status: { ...baseStatus, ...running, computer_state: "paused", viewport_state: "paused" },
      tasks: [...history, current("running")],
      events: [...action("t1", "screenshot", {}, 50), ...action("t1", "click", { x: 0.2, y: 0.4 }, 40)],
    };
    case "local-ready": case "local-running":
      return { status: { ...baseStatus, ...running }, tasks: history, events: [] };
    case "cloud":
      return { status: { ...baseStatus, provider: "anthropic" }, tasks: [...history, current("pending")], events: [{ type: "task_created", task_id: "t1", title: "Find duplicate files in Downloads and clean them up", at: iso(95) }] };
    default:
      // Pegoles Local not ready yet: a task waits for it.
      return { status: baseStatus, tasks: [...history, current("pending")], events: [{ type: "task_created", task_id: "t1", title: "Find duplicate files in Downloads and clean them up", at: iso(95) }] };
  }
}

// ── Pegoles Local (simulated model store and setup job) ────────────────
/** Mirrors the compiled catalog (crates/pegoles-inference/catalog/models.json) for previews only. */
const CATALOG: readonly LocalModelInfo[] = [
  { id: "mai-ui-2b-6bit", display_name: "MAI-UI 2B", family: "MaiUi", parameters: "2B", quantization: "6bit", size_bytes: 2_226_454_187, license: "apache-2.0",
    source: "huggingface.co/mlx-community/MAI-UI-2B-6bit-v2 @ cb57cf2fc99f", state: "not_installed", partial_bytes: null, invalid_reason: null, downloadable: true, recommended_min_ram_gb: null },
  { id: "qwen3-vl-2b-6bit", display_name: "Qwen3-VL 2B Instruct", family: "Qwen3Vl", parameters: "2B", quantization: "6bit", size_bytes: 2_228_134_653, license: "apache-2.0",
    source: "huggingface.co/mlx-community/Qwen3-VL-2B-Instruct-6bit @ 0b20c3743b11", state: "not_installed", partial_bytes: null, invalid_reason: null, downloadable: true, recommended_min_ram_gb: null },
  { id: "qwen3-vl-2b-4bit", display_name: "Qwen3-VL 2B Instruct", family: "Qwen3Vl", parameters: "2B", quantization: "4bit", size_bytes: 1_798_021_605, license: "apache-2.0",
    source: "huggingface.co/mlx-community/Qwen3-VL-2B-Instruct-4bit @ 9c4f5209e57b", state: "not_installed", partial_bytes: null, invalid_reason: null, downloadable: true, recommended_min_ram_gb: null },
];
const DEFAULT_MODEL = CATALOG[0].id;
const TICK_MS = 250;
/** A new setup downloads in this many ticks (12 s). */
const DOWNLOAD_TICKS = 48;

interface LocalWorld {
  provider: Provider;
  localModel: string;
  models: LocalModelInfo[];
  install: ModelInstallStatus | null;
  appleSilicon: boolean;
  runtimeReady: boolean;
  loaded: string | null;
  footprint: number | null;
  /** Bytes per tick of the scenario's own download (a new setup runs at DOWNLOAD_TICKS). */
  rate: number;
}

function localWorld(scenario: Scenario, ready: boolean): LocalWorld {
  const size = CATALOG[0].size_bytes;
  const job = (phase: ModelInstallStatus["phase"], done: number, extra: Partial<ModelInstallStatus> = {}): ModelInstallStatus =>
    ({ model: DEFAULT_MODEL, phase, done_bytes: done, total_bytes: size, error: null, error_kind: null, ...extra });
  const models = (patch: Partial<LocalModelInfo> = {}) => CATALOG.map((model, i) => (i === 0 ? { ...model, ...patch } : { ...model }));
  const lw: LocalWorld = {
    provider: "local", localModel: DEFAULT_MODEL, models: models(ready ? { state: "installed" } : {}), install: null,
    appleSilicon: true, runtimeReady: true, loaded: null, footprint: null, rate: size / DOWNLOAD_TICKS,
  };
  switch (scenario) {
    case "local-downloading":
      return { ...lw, models: models({ state: "partial", partial_bytes: Math.round(size * 0.42) }), install: job("downloading", Math.round(size * 0.42)), rate: size / 400 };
    case "local-verifying": return { ...lw, models: models({ state: "partial", partial_bytes: size }), install: job("verifying", size) };
    case "local-ready": return { ...lw, models: models({ state: "installed" }) };
    case "local-running": return { ...lw, models: models({ state: "installed" }), loaded: DEFAULT_MODEL, footprint: 2_463_000_000 };
    case "local-failed": return {
      ...lw, models: models({ state: "partial", partial_bytes: 0 }),
      install: job("failed", 0, { error: "Not enough disk space: Pegoles Local needs 2.4 GB free and 1.1 GB is available.", error_kind: "disk_space" }),
    };
    case "local-paused": return { ...lw, models: models({ state: "partial", partial_bytes: Math.round(size * 0.37) }), install: job("cancelled", 0) };
    case "local-damaged": return { ...lw, models: models({ state: "invalid", invalid_reason: "model.safetensors has the wrong size" }) };
    case "local-unsupported": return { ...lw, appleSilicon: false, runtimeReady: false };
    case "cloud": return { ...lw, provider: "anthropic" };
    default: return lw;
  }
}

type Handler = (event: { event: string; id: number; payload: unknown }) => void;

/** A drawn stand-in for the guest's framebuffer, so the snapshot path can be QA'd without a VM. */
function fakeScreen(): string {
  const canvas = document.createElement("canvas");
  canvas.width = 1440;
  canvas.height = 900;
  const g = canvas.getContext("2d");
  if (!g) return "";
  const wall = g.createLinearGradient(0, 0, 1440, 900);
  wall.addColorStop(0, "#1d2a44");
  wall.addColorStop(1, "#0d1220");
  g.fillStyle = wall;
  g.fillRect(0, 0, 1440, 900);
  g.fillStyle = "#11151d";
  g.fillRect(0, 0, 1440, 34);
  g.fillStyle = "#c9d1e0";
  g.font = "600 15px sans-serif";
  g.fillText("Activities", 18, 22);
  g.fillText("Sat 00:42", 680, 22);
  // A browser window.
  g.fillStyle = "#f4f5f7";
  g.beginPath(); g.roundRect(150, 80, 1140, 740, 12); g.fill();
  g.fillStyle = "#e3e6eb";
  g.beginPath(); g.roundRect(150, 80, 1140, 52, [12, 12, 0, 0]); g.fill();
  g.fillStyle = "#ffffff";
  g.beginPath(); g.roundRect(330, 92, 780, 28, 14); g.fill();
  g.fillStyle = "#5a6270";
  g.font = "15px sans-serif";
  g.fillText("notion.so/pricing", 350, 111);
  g.fillStyle = "#111318";
  g.font = "700 40px sans-serif";
  g.fillText("Pricing that grows with you", 230, 220);
  g.fillStyle = "#6b7280";
  g.font = "20px sans-serif";
  g.fillText("Compare plans and features", 230, 258);
  [0, 1, 2].forEach((i) => {
    const x = 230 + i * 345;
    g.fillStyle = i === 1 ? "#eef4ff" : "#f7f8fa";
    g.beginPath(); g.roundRect(x, 300, 320, 420, 14); g.fill();
    g.strokeStyle = i === 1 ? "#2f7cf6" : "#e1e4ea"; g.lineWidth = 2; g.stroke();
    g.fillStyle = "#111318"; g.font = "600 24px sans-serif";
    g.fillText(["Free", "Plus", "Business"][i], x + 24, 350);
    g.font = "700 44px sans-serif";
    g.fillText(["$0", "$10", "$15"][i], x + 24, 420);
    g.fillStyle = "#8a909c";
    for (let r = 0; r < 5; r += 1) { g.fillRect(x + 24, 470 + r * 36, 200 - r * 18, 10); }
    g.fillStyle = i === 1 ? "#2f7cf6" : "#111318";
    g.beginPath(); g.roundRect(x + 24, 650, 272, 44, 10); g.fill();
  });
  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}

export function installShellLab(hash: string): void {
  const [, , , scenario, place] = hash.split("/");
  const name = (scenario || "home") as Scenario;
  const state = world(name);
  // As in Core: a running task is the one agent run.
  const live = state.tasks.find((candidate) => candidate.status === "running");
  if (live && !state.status.active_task) state.status = { ...state.status, active_task: live.id };
  const callbacks = new Map<number, Handler>();
  const listeners = new Map<number, { event: string; handler: number }>();
  let nextId = 1;
  let model: ModelSettings = {
    configured: false, key_source: null,
    model: "claude-opus-5", effort: "high", models: ["claude-opus-5", "claude-sonnet-5", "claude-opus-5-5"], efforts: ["low", "medium", "high", "xhigh", "max"],
  };
  const lw = localWorld(name, state.status.model === "configured");
  if (state.tasks.some((candidate) => candidate.status === "running") && lw.models[0].state === "installed") {
    lw.loaded = lw.localModel;
    lw.footprint = 2_463_000_000;
  }

  const chosen = () => lw.models.find((candidate) => candidate.id === lw.localModel) ?? lw.models[0];
  /** As Core's refresh_readiness: the chosen provider can run a task now. */
  const readiness = () => {
    const ready = lw.provider === "local" ? lw.runtimeReady && chosen().state === "installed" : model.configured;
    state.status = { ...state.status, provider: lw.provider, model: ready ? "configured" : "not_configured" };
  };
  readiness();
  const intelligence = (): Intelligence => ({
    provider: lw.provider, local_model: lw.localModel, anthropic: model,
    local: {
      runtime_ready: lw.runtimeReady && lw.appleSilicon,
      runtime_problem: lw.appleSilicon ? (lw.runtimeReady ? null : "local model runtime is not installed: the Pegoles Local runtime is not set up on this Mac") : "Pegoles Local needs a Mac with Apple silicon.",
      loaded_model: lw.loaded, worker_footprint_bytes: lw.footprint, default_model: DEFAULT_MODEL,
      models: lw.models.map((candidate) => ({ ...candidate })), install: lw.install,
      chip: lw.appleSilicon ? "Apple M3 Pro" : "Intel(R) Core(TM) i9-9880H CPU @ 2.30GHz", memory_bytes: 18 * 2 ** 30, apple_silicon: lw.appleSilicon,
    },
  });
  const emit = (payload: ModelInstallStatus) => {
    lw.install = payload;
    for (const [id, listener] of listeners) {
      if (listener.event === MODEL_INSTALL_EVENT) callbacks.get(listener.handler)?.({ event: MODEL_INSTALL_EVENT, id, payload });
    }
  };
  const patchModel = (id: string, patch: Partial<LocalModelInfo>) => {
    lw.models = lw.models.map((candidate) => (candidate.id === id ? { ...candidate, ...patch } : candidate));
  };

  // One simulated setup job at a time, like LocalModels::start_install.
  let job: { model: string; timer: number | null; done: number } | null = null;
  const settle = (payload: ModelInstallStatus) => {
    if (job?.timer) window.clearInterval(job.timer);
    job = null;
    emit(payload);
    readiness();
  };
  const run = (id: string, rate: number) => {
    const spec = lw.models.find((candidate) => candidate.id === id) ?? chosen();
    const total = spec.size_bytes;
    const status = (phase: ModelInstallStatus["phase"], done: number): ModelInstallStatus =>
      ({ model: spec.id, phase, done_bytes: done, total_bytes: total, error: null, error_kind: null });
    const current = { model: spec.id, timer: null as number | null, done: spec.state === "partial" ? spec.partial_bytes ?? 0 : 0 };
    job = current;
    patchModel(spec.id, { state: "partial", partial_bytes: current.done });
    lw.install = status(current.done >= total ? "verifying" : "downloading", current.done);
    const finish = () => {
      window.setTimeout(() => { if (job === current) emit(status("finalizing", total)); }, 1800);
      window.setTimeout(() => {
        if (job !== current) return;
        patchModel(spec.id, { state: "installed", partial_bytes: null });
        settle(status("ready", total));
      }, 4200);
    };
    if (current.done >= total) { finish(); return; }
    current.timer = window.setInterval(() => {
      current.done = Math.min(total, current.done + rate);
      patchModel(spec.id, { partial_bytes: Math.round(current.done) });
      if (current.done < total) { emit(status("downloading", Math.round(current.done))); return; }
      if (current.timer) window.clearInterval(current.timer);
      current.timer = null;
      emit(status("verifying", total));
      finish();
    }, TICK_MS);
  };
  // A scenario that opens mid-setup keeps going, at its own pace.
  if (lw.install && installActive(lw.install)) run(lw.install.model, lw.rate);

  const setModel = (next: ModelSettings) => {
    model = next;
    readiness();
    return model;
  };
  const setTask = (id: string, status: AgentTask["status"]) => {
    const previous = state.tasks.find((candidate) => candidate.id === id);
    state.tasks = state.tasks.map((candidate) => candidate.id === id ? { ...candidate, status, updated_at: new Date().toISOString() } : candidate);
    if (previous) state.events = [...state.events, { type: "task_status_changed", task_id: id, from: previous.status, to: status, at: new Date().toISOString() }];
  };
  const say = (taskId: string, kind: AgentMessageKind, text: string) => { state.events = [...state.events, note(taskId, kind, text, 0)]; };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_status: () => state.status,
    list_events: () => state.events,
    list_tasks: () => state.tasks,
    get_model_settings: () => model,
    // Core asks for the key in a macOS window; the lab stands in for a
    // person who pastes a valid one.
    enter_api_key: () => setModel({ ...model, configured: true, key_source: "keychain" }),
    clear_api_key: () => setModel({ ...model, configured: false, key_source: null }),
    set_model_settings: (args) => setModel({ ...model, model: String(args.model), effort: String(args.effort) }),
    get_intelligence: () => { readiness(); return intelligence(); },
    set_provider: (args) => {
      lw.provider = args.provider === "anthropic" ? "anthropic" : "local";
      if (typeof args.localModel === "string") lw.localModel = args.localModel;
      readiness();
      return intelligence();
    },
    install_local_model: (args) => {
      const id = typeof args.model === "string" ? args.model : lw.localModel;
      if (job) throw `${job.model} is already being set up`;
      const spec = lw.models.find((candidate) => candidate.id === id);
      if (!spec) throw `unknown model "${id}"`;
      run(id, spec.size_bytes / DOWNLOAD_TICKS);
      return intelligence();
    },
    cancel_local_model_install: () => {
      const current = job;
      if (current?.timer) window.clearInterval(current.timer);
      if (current) {
        // As in Core: the flag is set now, the job says "cancelled" a moment later.
        const spec = lw.models.find((candidate) => candidate.id === current.model) ?? chosen();
        window.setTimeout(() => settle({ model: spec.id, phase: "cancelled", done_bytes: 0, total_bytes: spec.size_bytes, error: null, error_kind: null }), 300);
      }
      return intelligence();
    },
    remove_local_model: (args) => {
      const id = String(args.model);
      if (state.status.active_task) throw "Stop the running task first.";
      if (job?.model === id) throw "cancel the download first";
      patchModel(id, { state: "not_installed", partial_bytes: null, invalid_reason: null });
      lw.install = null;
      if (lw.loaded === id) { lw.loaded = null; lw.footprint = null; }
      readiness();
      return intelligence();
    },
    run_task: (args) => {
      const id = String(args.taskId);
      readiness();
      if (state.status.model !== "configured") {
        throw lw.provider === "anthropic" ? "Connect a model in Settings first: add an Anthropic key or switch to Pegoles Local." : "Set up Pegoles Local in Settings first.";
      }
      if (state.status.active_task) throw `task ${state.status.active_task} is already running`;
      const target = state.tasks.find((candidate) => candidate.id === id);
      if (target?.status !== "pending") throw `task is ${target?.status ?? "missing"}, not pending`;
      if (lw.provider === "local") { lw.loaded = lw.localModel; lw.footprint = 2_463_000_000; }
      state.status = { ...state.status, ...running, model: "configured", active_task: id };
      setTask(id, "running");
      say(id, "progress", "Looking at the screen first to see where things are.");
      return null;
    },
    cancel_task: (args) => {
      const id = String(args.taskId);
      if (state.status.active_task === id) state.status = { ...state.status, active_task: null, agent_busy: false, control_owner: "none", viewport_state: "ready" };
      setTask(id, "cancelled");
      say(id, "error", "Stopped by the user.");
      return null;
    },
    reset_computer: () => {
      if (state.status.active_task) throw "stop the running task before resetting the computer";
      return { info: null };
    },
    destroy_computer: () => {
      if (state.status.active_task) throw "stop the running task before removing the computer";
      state.status = { ...state.status, computer_created: false, computer_state: null, computer_id: null, viewport_state: "off", guest_state: "unavailable", control_owner: "none" };
      return { info: null };
    },
    get_host_capabilities: () => ({ platform: "macos", architecture: "arm64", backend: "real", backend_available: true, backend_detail: "", guest_transport: "virtio_socket", guest_transport_available: true, required_setup: [], supported: true }),
    suggested_effects: () => ({ tier: "full" }),
    create_task: (args) => {
      const created = task(`n${nextId++}`, String(args.title), "pending", 0);
      state.tasks = [...state.tasks, created];
      state.events = [...state.events, { type: "task_created", task_id: created.id, title: created.title, at: created.created_at }];
      return created;
    },
    create_computer: () => { if (state.failComputer) throw state.failComputer; state.status = { ...state.status, computer_created: true, computer_state: "stopped" }; return { info: null }; },
    start_computer: () => { if (state.failComputer) throw state.failComputer; state.status = { ...state.status, ...running, model: state.status.model }; return { info: null }; },
    stop_computer: () => { state.status = { ...state.status, computer_state: "stopped", viewport_state: "off", control_owner: "none" }; return { info: null }; },
    pause_computer: () => { state.status = { ...state.status, computer_state: "paused", viewport_state: "paused" }; return { info: null }; },
    resume_computer: () => { state.status = { ...state.status, computer_state: "running", viewport_state: "ready" }; return { info: null }; },
    take_control: () => { state.status = { ...state.status, control_owner: "user", viewport_state: "user_controlled" }; return state.status; },
    return_control: () => { state.status = { ...state.status, control_owner: "agent", viewport_state: "agent_active" }; return state.status; },
    display_set_geometry: () => ({ outcome: "ok" }),
    display_detach: () => null,
    capture_screen: () => {
      if (state.status.computer_state !== "running") throw new Error("computer is not running");
      return { meta: { frame_id: "lab", computer_id: "vm-1", captured_at: new Date().toISOString(), width_px: 1440, height_px: 900, encoding: "png", byte_len: 0, capture_latency_ms: 12 }, png_base64: fakeScreen() };
    },
    cancel_agent_input: () => null,
    read_boot_log: () => ({ available: true, total_lines: 3, tail: ["[    0.000000] Booting Linux on physical CPU 0x0", "[    1.204118] systemd[1]: Reached target graphical.target", "pegoles-guest: runtime ready (protocol 2)"] }),
    "plugin:event|listen": (args) => {
      const id = nextId++;
      listeners.set(id, { event: String(args.event), handler: Number(args.handler) });
      return id;
    },
    "plugin:event|unlisten": (args) => { listeners.delete(Number(args.eventId)); return null; },
  };

  const target = window as unknown as Record<string, unknown>;
  target.isTauri = true;
  target.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
  target.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      const handler = commands[cmd];
      if (!handler) throw new Error(`shell lab: unhandled command ${cmd}`);
      return handler(args);
    },
    transformCallback: (callback: Handler) => { const id = nextId++; callbacks.set(id, callback); return id; },
    unregisterCallback: (id: number) => { callbacks.delete(id); },
    convertFileSrc: (path: string) => path,
  };
  if (place === "settings") openIntelligenceSettings();
}

/** `/settings`: arrive at Settings → Intelligence the way a person would, through the composer's model chip. */
function openIntelligenceSettings(attempt = 0): void {
  const chip = [...document.querySelectorAll<HTMLButtonElement>(".composer-chip")].find((button) => /Open settings$|^Model:/.test(button.getAttribute("aria-label") ?? ""));
  if (chip) { chip.click(); return; }
  if (attempt < 50) window.setTimeout(() => openIntelligenceSettings(attempt + 1), 100);
}

/**
 * DEV ONLY — never shipped (main.tsx gates it on import.meta.env.DEV and the
 * production bundle test checks for the marker below).
 *
 * Installs a simulated Tauri bridge so the REAL App can be rendered in every
 * product state for visual QA: `#/dev/shell/<scenario>`. Nothing here is
 * reachable from production code paths.
 */
import type { AgentEvent, AgentMessageKind, AgentTask, ModelSettings, StatusPayload } from "../lib/tauri";

export const SHELL_LAB_MARKER = "__PEGOLES_SHELL_LAB__";

type Scenario =
  | "home" | "pending" | "ready" | "thinking" | "running" | "files" | "computer" | "user" | "approval"
  | "done" | "cancelled" | "failure" | "long" | "booting" | "offline-setup" | "paused";

const now = Date.now();
const iso = (secondsAgo: number) => new Date(now - secondsAgo * 1000).toISOString();

const baseStatus: StatusPayload = {
  core: "running", model: "not_configured", backend: "real", computer_created: false, computer_state: null, computer_id: null,
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
  const name = (hash.split("/")[3] ?? "home") as Scenario;
  const state = world(name);
  // As in Core: a running task is the one agent run.
  const live = state.tasks.find((candidate) => candidate.status === "running");
  if (live && !state.status.active_task) state.status = { ...state.status, active_task: live.id };
  const callbacks = new Map<number, Handler>();
  let nextId = 1;
  let model: ModelSettings = {
    configured: state.status.model === "configured", key_source: state.status.model === "configured" ? "keychain" : null,
    model: "claude-opus-5", effort: "high", models: ["claude-opus-5", "claude-sonnet-5", "claude-opus-5-5"], efforts: ["low", "medium", "high", "xhigh", "max"],
  };
  const setModel = (next: ModelSettings) => {
    model = next;
    state.status = { ...state.status, model: next.configured ? "configured" : "not_configured" };
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
    set_api_key: (args) => {
      if (String(args.key ?? "").trim().length < 20) throw "the key should be 20 to 256 characters";
      return setModel({ ...model, configured: true, key_source: "keychain" });
    },
    clear_api_key: () => setModel({ ...model, configured: false, key_source: null }),
    set_model_settings: (args) => setModel({ ...model, model: String(args.model), effort: String(args.effort) }),
    run_task: (args) => {
      const id = String(args.taskId);
      if (state.status.model !== "configured") throw "Connect a model in Settings first.";
      if (state.status.active_task) throw `task ${state.status.active_task} is already running`;
      const target = state.tasks.find((candidate) => candidate.id === id);
      if (target?.status !== "pending") throw `task is ${target?.status ?? "missing"}, not pending`;
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
    "plugin:event|listen": () => nextId++,
    "plugin:event|unlisten": () => null,
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
}

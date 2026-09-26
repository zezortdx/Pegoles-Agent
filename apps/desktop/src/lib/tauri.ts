import type { EffectsTier, ViewportState, TaskStatusWire } from "@pegoles/ui";
import { invoke } from "@tauri-apps/api/core";

export type ComputerState = "stopped" | "starting" | "running" | "paused" | "stopping" | "error";

export type GuestRuntimeState =
  | "unavailable"
  | "waiting"
  | "connecting"
  | "ready"
  | "disconnected"
  | "incompatible"
  | "error";

export interface StatusPayload {
  viewport_state: ViewportState;
  viewport_issue: string | null;
  display_available: boolean;
  display_attached: boolean;
  display_config: { width_px: number; height_px: number } | null;
  display_error: string | null;
  display_setup_error: string | null;
  control_owner: "none" | "agent" | "user";
  core: string;
  /** An Anthropic API key is available (Keychain or ANTHROPIC_API_KEY). */
  model: "configured" | "not_configured";
  /** The task an agent run is working on right now, if any. */
  active_task: string | null;
  backend: "mock" | "real";
  computer_created: boolean;
  computer_state: ComputerState | null;
  computer_id: string | null;
  /**
   * The sealed Pegoles computer image on this Mac. `missing`: not installed
   * (built with scripts/build-guest-image); `invalid`: present but not verified.
   */
  image_status: "missing" | "downloading" | "ready" | "invalid";
  spec_os: string;
  spec_arch: string;
  spec_vcpus: number;
  spec_ram_mb: number;
  guest_state: GuestRuntimeState;
  guest_ready_ms: number | null;
  input_available: boolean;
  agent_busy: boolean;
}

export interface ComputerInfo {
  id: string;
  state: ComputerState;
  config: {
    vcpus: number;
    memory_mb: number;
    disk_gb: number;
    workspace_root: string;
  };
}

export interface ComputerPayload {
  info: ComputerInfo | null;
}

export interface BootLogPayload {
  available: boolean;
  total_lines: number;
  tail: string[];
}

export interface HostCapabilities {
  platform: "macos" | "windows" | "linux";
  architecture: "arm64" | "x86_64";
  backend: "mock" | "real" | "windows-hcs";
  backend_available: boolean;
  backend_detail: string;
  guest_transport: "unavailable" | "virtio_socket" | "hyperv_socket";
  guest_transport_available: boolean;
  required_setup: string[];
  supported: boolean;
}

// AgentEvent wire format (serde tag = "type", snake_case).
// Phase 5 additions carry the FULL ActionRequest for cursor/UI mapping;
// secret typing text arrives redacted (empty + sensitive flag).
export interface ActionRequestWire {
  action_id: string;
  task_id: string;
  computer_id: string;
  action: { type: string; [k: string]: unknown };
  observe_after?: boolean;
  requested_at: string;
}
export interface ActionResultWire {
  action_id: string;
  outcome: string;
  success: boolean;
  message: string;
  duration_ms: number;
  error?: string | null;
  resulting_frame_id?: string | null;
}
export interface FrameMetaWire {
  frame_id: string;
  computer_id: string;
  captured_at: string;
  width_px: number;
  height_px: number;
  encoding: string;
  byte_len: number;
  capture_latency_ms: number;
}
/** What an `agent_message` carries: notes between actions, the final account, or why a run stopped. */
export type AgentMessageKind = "progress" | "summary" | "error";

export type AgentEvent =
  | { type: "task_created"; task_id: string; title: string; at: string }
  | { type: "task_status_changed"; task_id: string; from: string; to: string; at: string }
  /** Model narration for a task. Untrusted text: rendered as plain text only, never markup. */
  | { type: "agent_message"; task_id: string; kind: AgentMessageKind; text: string; at: string }
  | { type: "computer_created"; computer_id: string; at: string }
  | {
      type: "computer_state_changed";
      computer_id: string;
      from: ComputerState;
      to: ComputerState;
      at: string;
    }
  | { type: "guest_runtime_waiting"; computer_id: string; at: string }
  | { type: "guest_runtime_connected"; computer_id: string; at: string }
  | {
      type: "guest_handshake_completed";
      computer_id: string;
      protocol_version: number;
      at: string;
    }
  | { type: "guest_runtime_ready"; computer_id: string; ready_in_ms: number; at: string }
  | { type: "guest_runtime_disconnected"; computer_id: string; reason: string; at: string }
  | {
      type: "guest_runtime_incompatible";
      computer_id: string;
      guest_version: number;
      at: string;
    }
  | { type: "guest_runtime_error"; computer_id: string; message: string; at: string }
  | { type: "action_requested"; request: ActionRequestWire }
  | {
      type: "action_evaluated";
      request: ActionRequestWire;
      verdict: { decision: string; risk: string; reason: string };
    }
  | { type: "action_started"; action_id: string; request: ActionRequestWire; at: string }
  | {
      type: "action_completed";
      request: ActionRequestWire;
      result: ActionResultWire;
    }
  | { type: "action_denied"; request: ActionRequestWire; reason: string }
  | {
      type: "action_failed";
      action_id: string;
      request: ActionRequestWire;
      error: string;
      at: string;
    }
  | { type: "approval_requested"; task_id: string; reason: string; at: string }
  | {
      type: "frame_observed";
      computer_id: string;
      frame: FrameMetaWire;
      action_id?: string | null;
      at: string;
    }
  | {
      type: "input_capability_changed";
      computer_id: string;
      available: boolean;
      reason: string;
      at: string;
    }
  | {
      type: "control_ownership_changed";
      computer_id: string;
      from: string;
      to: string;
      at: string;
    }
  | { type: string; [k: string]: unknown };

export interface AgentTask {
  id: string;
  title: string;
  status: TaskStatusWire;
  created_at: string;
  updated_at: string;
}

export interface DisplayGeometry {
  rect: { x: number; y: number; width: number; height: number };
  visible: boolean;
  animate_ms: number;
}

/** Model settings as Core reports them. The API key itself never leaves Rust. */
export interface ModelSettings {
  readonly configured: boolean;
  readonly key_source: "keychain" | "environment" | null;
  readonly model: string;
  readonly effort: string;
  readonly models: readonly string[];
  readonly efforts: readonly string[];
}

export const api = {
  createTask: (title: string) => invoke<AgentTask>("create_task", { title }),
  listTasks: () => invoke<AgentTask[]>("list_tasks"),
  /** Start the agent on a pending task (it prepares its computer itself). */
  runTask: (taskId: string) => invoke<null>("run_task", { taskId }),
  /** Stop a running task, or cancel one that never started. */
  cancelTask: (taskId: string) => invoke<null>("cancel_task", { taskId }),
  getModelSettings: () => invoke<ModelSettings>("get_model_settings"),
  /** Stored in the macOS Keychain; never returned. */
  setApiKey: (key: string) => invoke<ModelSettings>("set_api_key", { key }),
  clearApiKey: () => invoke<ModelSettings>("clear_api_key"),
  setModelSettings: (model: string, effort: string) => invoke<ModelSettings>("set_model_settings", { model, effort }),
  suggestedEffects: () => invoke<{ tier: EffectsTier }>("suggested_effects"),
  setDisplayGeometry: (geometry: DisplayGeometry) => invoke("display_set_geometry", { geometry }),
  detachDisplay: () => invoke("display_detach"),
  takeControl: () => invoke("take_control"),
  returnControl: () => invoke("return_control"),
  getStatus: () => invoke<StatusPayload>("get_status"),
  createComputer: () => invoke<ComputerPayload>("create_computer"),
  startComputer: () => invoke<ComputerPayload>("start_computer"),
  pauseComputer: () => invoke<ComputerPayload>("pause_computer"),
  resumeComputer: () => invoke<ComputerPayload>("resume_computer"),
  stopComputer: () => invoke<ComputerPayload>("stop_computer"),
  /** Back to the sealed image: same identity, fresh disk. Refused while a task runs. */
  resetComputer: () => invoke<ComputerPayload>("reset_computer"),
  /** Removes the computer and its disk. Refused while a task runs. */
  destroyComputer: () => invoke<ComputerPayload>("destroy_computer"),
  listEvents: () => invoke<AgentEvent[]>("list_events"),
  readBootLog: () => invoke<BootLogPayload>("read_boot_log"),
  getHostCapabilities: () => invoke<HostCapabilities>("get_host_capabilities"),
  accessibilityDisplay: () => invoke<{ reduce_transparency: boolean; increase_contrast: boolean }>("accessibility_display"),
  /** Interrupts input in flight and stops any agent run. */
  cancelAgentInput: () => invoke("cancel_agent_input"),
  captureScreen: () =>
    invoke<{ meta: FrameMetaWire; png_base64: string }>("capture_screen"),
};

type WireAction = { type: string; [k: string]: unknown };
type ScriptStep = { label: string; action: WireAction; observe_after?: boolean };

/**
 * Design Lab only. `execute_action`, `run_input_script` and
 * `demo_script_steps` exist only in debug builds of Core; production code
 * never calls anything here.
 */
export const debugApi = {
  executeAction: (action: WireAction, taskId?: string, observeAfter?: boolean) =>
    invoke<ActionResultWire>("execute_action", {
      taskId: taskId ?? null,
      action,
      observeAfter: observeAfter ?? false,
    }),
  runInputScript: (steps: ScriptStep[], taskId?: string) =>
    invoke<{ steps_total: number; steps_executed: number; aborted_at: number | null; results: ActionResultWire[] }>(
      "run_input_script",
      { taskId: taskId ?? null, steps },
    ),
  demoScriptSteps: () => invoke<ScriptStep[]>("demo_script_steps"),
  inputStatus: () =>
    invoke<{
      available: boolean;
      frame_available: boolean;
      agent_busy: boolean;
      pressed_clean: boolean;
      audit_len: number;
      last_frame: FrameMetaWire | null;
    }>("input_status"),
};

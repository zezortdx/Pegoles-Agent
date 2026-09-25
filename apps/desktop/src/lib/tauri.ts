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
  model: string;
  backend: "mock" | "real";
  computer_created: boolean;
  computer_state: ComputerState | null;
  computer_id: string | null;
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

export interface ImageStatusPayload {
  status: "missing" | "downloading" | "ready" | "invalid";
  preparing: boolean;
  stage: string | null;
  downloaded: number;
  total: number;
  error: string | null;
}

export interface BootLogPayload {
  available: boolean;
  total_lines: number;
  tail: string[];
}

export interface SystemInfo {
  os: string;
  os_version: string;
  kernel: string;
  arch: string;
  hostname: string;
  runtime_version: string;
  protocol_version: number;
  uptime_s?: number;
  cpu_count?: number;
  mem_total_mb?: number;
}

export interface GuestInfoPayload {
  available: boolean;
  info: SystemInfo | null;
}

export interface GuestPingPayload {
  latency_ms: number;
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

export interface ImageProgressEvent {
  stage: string;
  downloaded?: number;
  total?: number;
  error?: string;
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
export type AgentEvent =
  | { type: "task_created"; task_id: string; title: string; at: string }
  | { type: "task_status_changed"; task_id: string; from: string; to: string; at: string }
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

export const api = {
  createTask: (title: string) => invoke<AgentTask>("create_task", { title }),
  listTasks: () => invoke<AgentTask[]>("list_tasks"),
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
  listEvents: () => invoke<AgentEvent[]>("list_events"),
  getImageStatus: () => invoke<ImageStatusPayload>("get_image_status"),
  prepareImage: () => invoke<ImageStatusPayload>("prepare_image"),
  readBootLog: () => invoke<BootLogPayload>("read_boot_log"),
  guestInfo: () => invoke<GuestInfoPayload>("guest_info"),
  guestPing: () => invoke<GuestPingPayload>("guest_ping"),
  getHostCapabilities: () => invoke<HostCapabilities>("get_host_capabilities"),
  accessibilityDisplay: () => invoke<{ reduce_transparency: boolean; increase_contrast: boolean }>("accessibility_display"),
  executeAction: (action: { type: string; [k: string]: unknown }, taskId?: string, observeAfter?: boolean) =>
    invoke<ActionResultWire>("execute_action", {
      taskId: taskId ?? null,
      action,
      observeAfter: observeAfter ?? false,
    }),
  cancelAgentInput: () => invoke("cancel_agent_input"),
  captureScreen: () =>
    invoke<{ meta: FrameMetaWire; png_base64: string }>("capture_screen"),
  runInputScript: (
    steps: { label: string; action: { type: string; [k: string]: unknown }; observe_after?: boolean }[],
    taskId?: string,
  ) =>
    invoke<{ steps_total: number; steps_executed: number; aborted_at: number | null; results: ActionResultWire[] }>(
      "run_input_script",
      { taskId: taskId ?? null, steps },
    ),
  demoScriptSteps: () =>
    invoke<{ label: string; action: { type: string; [k: string]: unknown }; observe_after?: boolean }[]>(
      "demo_script_steps",
    ),
  inputStatus: () =>
    invoke<{
      available: boolean;
      frame_available: boolean;
      agent_busy: boolean;
      pressed_clean: boolean;
      audit_len: number;
      last_frame: FrameMetaWire | null;
    }>("input_status"),
  inputAudit: (limit?: number) =>
    invoke<
      {
        at: string;
        action_id: string;
        verb: string;
        decision: string;
        outcome: string;
        duration_ms: number;
        redacted: boolean;
        text_len: number | null;
      }[]
    >("input_audit", { limit: limit ?? 50 }),
};

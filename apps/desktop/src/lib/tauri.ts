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
  /**
   * The chosen provider can run a task now: Pegoles Local is installed and
   * its runtime is ready, or (with Anthropic chosen) a key is available.
   */
  model: "configured" | "not_configured";
  /** Who plans: Pegoles Local on this Mac (the default) or Anthropic. */
  provider: Provider;
  /** The task an agent run is working on right now, if any. */
  active_task: string | null;
  backend: "mock" | "real";
  computer_created: boolean;
  computer_state: ComputerState | null;
  computer_id: string | null;
  /**
   * The sealed Pegoles computer image on this Mac. `missing`: not installed;
   * `invalid`: present but not verified.
   */
  image_status: "missing" | "downloading" | "ready" | "invalid";
  /** Setting that image up from its pinned download (in-app, one time). */
  image_setup?: ImageSetup;
  spec_os: string;
  spec_arch: string;
  spec_vcpus: number;
  spec_ram_mb: number;
  guest_state: GuestRuntimeState;
  guest_ready_ms: number | null;
  input_available: boolean;
  agent_busy: boolean;
}

/**
 * The Pegoles computer image is downloaded once and checked against the
 * digests built into the app. Real bytes only.
 */
export interface ImageSetup {
  /** This build has a download location for the image. */
  available: boolean;
  installing: boolean;
  stage: "downloading" | "verifying" | "unpacking" | "finalizing" | null;
  done: number;
  total: number;
  /** Why the last attempt stopped (cancelled, network, verification). */
  error: string | null;
  download_bytes: number;
  disk_bytes: number;
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

/** Who plans: Pegoles Local (on this Mac, the default) or Anthropic (cloud, optional). */
export type Provider = "local" | "anthropic";

/** One model Pegoles Local can run, as Core's catalog and model store report it. */
export interface LocalModelInfo {
  readonly id: string;
  readonly display_name: string;
  readonly family: string;
  readonly parameters: string;
  readonly quantization: string;
  readonly size_bytes: number;
  readonly license: string;
  readonly source: string;
  readonly state: "not_installed" | "partial" | "installed" | "invalid";
  /** Bytes already on disk while `state` is `partial` (a retry resumes from here). */
  readonly partial_bytes: number | null;
  readonly invalid_reason: string | null;
  readonly downloadable: boolean;
  readonly recommended_min_ram_gb: number | null;
}

export type InstallPhase = "idle" | "downloading" | "verifying" | "finalizing" | "ready" | "failed" | "cancelled";
export type InstallErrorKind = "disk_space" | "network" | "corrupted" | "other";

/** The latest local-model setup job this session (also the `pegoles://model-install` payload). */
export interface ModelInstallStatus {
  readonly model: string;
  readonly phase: InstallPhase;
  readonly done_bytes: number;
  readonly total_bytes: number;
  /** Plain language, safe to show as text. */
  readonly error: string | null;
  readonly error_kind: InstallErrorKind | null;
}

export interface LocalRuntime {
  /** The inference runtime is present on this Mac. */
  readonly runtime_ready: boolean;
  /** Plain-language reason when it isn't. */
  readonly runtime_problem: string | null;
  /** The model loaded in the worker right now (running locally). */
  readonly loaded_model: string | null;
  /** Measured memory of the running worker. */
  readonly worker_footprint_bytes: number | null;
  readonly default_model: string;
  readonly models: readonly LocalModelInfo[];
  readonly install: ModelInstallStatus | null;
  readonly chip: string | null;
  readonly memory_bytes: number;
  readonly apple_silicon: boolean;
}

/** Everything about who plans, as Core reports it. Never contains a secret. */
export interface Intelligence {
  readonly provider: Provider;
  /** The chosen local model (the catalog default until someone picks another). */
  readonly local_model: string;
  readonly local: LocalRuntime;
  readonly anthropic: ModelSettings;
}

/** Emitted ~4×/s while downloading and on every phase change. */
export const MODEL_INSTALL_EVENT = "pegoles://model-install";

/** The first-run screens, in order. */
export type OnboardingStep = "welcome" | "how" | "check" | "setup" | "intelligence" | "ready";

/** Where the person is in onboarding (Core persists it across restarts). */
export interface OnboardingState {
  readonly version: number;
  readonly step: OnboardingStep;
  readonly completed: boolean;
  /** They chose to restart Windows to finish turning virtualization on. */
  readonly restart_requested: boolean;
}

export type VirtualizationState =
  | "ready" | "needs_enable" | "restart_pending" | "firmware_disabled" | "unsupported" | "unknown";
export type AccelerationKind = "metal" | "cuda" | "vulkan" | "cpu" | "none";

/** What Core found about this computer (facts; the UI words them). */
export interface SystemCheck {
  readonly platform: "macos" | "windows" | "linux";
  readonly os_name: string;
  readonly os_supported: boolean;
  readonly os_minimum: string;
  readonly architecture: "arm64" | "x86_64";
  readonly architecture_supported: boolean;
  readonly virtualization: { readonly state: VirtualizationState; readonly fixable: boolean; readonly technical: string };
  readonly memory_bytes: number;
  readonly memory_minimum_bytes: number;
  readonly memory_recommended_bytes: number;
  readonly disk_free_bytes: number | null;
  readonly disk_needed_bytes: number;
  readonly acceleration: { readonly kind: AccelerationKind; readonly device: string | null; readonly technical: string };
  readonly runtime_ready: boolean;
  readonly runtime_problem: string | null;
  readonly model_ready: boolean;
  readonly image_ready: boolean;
}

/** After asking Windows to turn virtualization on. */
export type FixOutcome = "ready" | "restart_required" | "declined";

export const api = {
  createTask: (title: string) => invoke<AgentTask>("create_task", { title }),
  listTasks: () => invoke<AgentTask[]>("list_tasks"),
  /** Start the agent on a pending task (it prepares its computer itself). */
  runTask: (taskId: string) => invoke<null>("run_task", { taskId }),
  /** Stop a running task, or cancel one that never started. */
  cancelTask: (taskId: string) => invoke<null>("cancel_task", { taskId }),
  getModelSettings: () => invoke<ModelSettings>("get_model_settings"),
  /**
   * Opens a macOS dialog where the person pastes their key. Core stores it in
   * the Keychain; it never passes through this web view and is never returned.
   * Rejects when the dialog is cancelled.
   */
  enterApiKey: () => invoke<ModelSettings>("enter_api_key"),
  clearApiKey: () => invoke<ModelSettings>("clear_api_key"),
  setModelSettings: (model: string, effort: string) => invoke<ModelSettings>("set_model_settings", { model, effort }),
  getIntelligence: () => invoke<Intelligence>("get_intelligence"),
  /**
   * Choose who plans; `localModel` also changes the chosen local model.
   * Moving to Anthropic waits for the person to confirm in a macOS dialog,
   * and rejects when they don't.
   */
  setProvider: (provider: Provider, localModel?: string) =>
    invoke<Intelligence>("set_provider", { provider, localModel: localModel ?? null }),
  /** Download, verify and install a local model (default: the chosen one). Resumes a partial download. */
  installLocalModel: (model?: string) => invoke<Intelligence>("install_local_model", { model: model ?? null }),
  cancelLocalModelInstall: () => invoke<Intelligence>("cancel_local_model_install"),
  /** Removes the model and any partial download. Refused while a task runs. */
  removeLocalModel: (model: string) => invoke<Intelligence>("remove_local_model", { model }),
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
  /** Download, verify and install the computer image. Returns at once; resumes a partial download. */
  installComputerImage: () => invoke<ImageSetup>("install_computer_image"),
  cancelComputerImageInstall: () => invoke<ImageSetup>("cancel_computer_image_install"),
  listEvents: () => invoke<AgentEvent[]>("list_events"),
  readBootLog: () => invoke<BootLogPayload>("read_boot_log"),
  getHostCapabilities: () => invoke<HostCapabilities>("get_host_capabilities"),
  accessibilityDisplay: () => invoke<{ reduce_transparency: boolean; increase_contrast: boolean }>("accessibility_display"),
  /** Interrupts input in flight and stops any agent run. */
  cancelAgentInput: () => invoke("cancel_agent_input"),
  captureScreen: () =>
    invoke<{ meta: FrameMetaWire; png_base64: string }>("capture_screen"),
  getOnboarding: () => invoke<OnboardingState>("get_onboarding"),
  setOnboardingStep: (step: OnboardingStep) => invoke<OnboardingState>("set_onboarding_step", { step }),
  finishOnboarding: () => invoke<OnboardingState>("finish_onboarding"),
  systemCheck: () => invoke<SystemCheck>("system_check"),
  /** Windows only: asks for administrator approval in Windows' own prompt. */
  fixVirtualization: () => invoke<FixOutcome>("fix_virtualization"),
  /** Windows only: restarts the PC (after the person pressed "Restart now"). */
  restartToFinishSetup: () => invoke<null>("restart_to_finish_setup"),
};

type WireAction = { type: string; [k: string]: unknown };
type ScriptStep = { label: string; action: WireAction; observe_after?: boolean };

/**
 * Design Lab only. These commands exist, and are granted to the web view,
 * only in debug builds of Core; production code never calls anything here.
 * Every command in `api` above, and only those, is granted to the release
 * web view (src-tauri/capabilities/default.json, checked by its tests).
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

import type { AgentEvent, ImageSetup, StatusPayload } from "../lib/tauri";
import { formatBytes } from "../lib/format";

/**
 * Pegoles Computer as people see it. Pure: derived only from Core's
 * status and boot events; the UI never guesses a state.
 */
export type ComputerPhase =
  | "unavailable" | "needs-setup" | "off" | "starting"
  | "ready" | "agent" | "user" | "paused" | "stopping" | "error";

/** `reset` returns it to the sealed image; `remove` deletes it and its disk. */
export type ComputerCommand =
  | "start" | "pause" | "resume" | "stop" | "take" | "return" | "reset" | "remove"
  /** Download, verify and install the computer image; or stop doing so. */
  | "install" | "cancel-install";

export interface ComputerAction {
  readonly command: ComputerCommand;
  readonly label: string;
}

/** A plain fact about the machine, always true of the current status. */
export interface ComputerFact {
  readonly label: string;
  readonly value: string;
}

export interface BootStep {
  readonly label: string;
  readonly state: "done" | "active" | "pending";
}

export interface ComputerModel {
  readonly phase: ComputerPhase;
  /** One word or two for the status chip. */
  readonly chip: string;
  /** Headline inside the preview when there is no screen to show. */
  readonly headline: string;
  readonly body?: string;
  /** A line for developers (how to fix a missing image), shown under the body. Dev builds only. */
  readonly devNote?: string;
  readonly primary?: ComputerAction;
  /** Less frequent actions, disclosed in an overflow. */
  readonly secondary: readonly ComputerAction[];
  readonly owner: "none" | "agent" | "user";
  /** The machine is up (a screen could exist). */
  readonly running: boolean;
  /** Something is in motion (setup, boot): the chip may animate. */
  readonly transitioning: boolean;
  readonly steps: readonly BootStep[];
  /** Technical facts, disclosed under Details only. */
  readonly specs: readonly string[];
  /** A few human facts shown in the panel so it is never a void. */
  readonly facts: readonly ComputerFact[];
}

export interface ComputerInputs {
  readonly connected: boolean;
  readonly native: boolean;
  readonly status: StatusPayload | null;
  readonly events: readonly AgentEvent[];
}

const BOOT_LABELS = ["Preparing isolated environment", "Connecting", "Starting display"] as const;

function bootSteps(status: StatusPayload, events: readonly AgentEvent[]): BootStep[] {
  const guestUp = status.guest_state === "ready" || events.some((event) => event.type === "guest_runtime_ready");
  const connecting = status.viewport_state === "guest_connecting" || status.guest_state === "connecting" || status.guest_state === "waiting";
  const display = status.viewport_state === "display_starting";
  const reached = display || guestUp ? 2 : connecting ? 1 : 0;
  return BOOT_LABELS.map((label, index) => ({
    label,
    state: index < reached ? "done" : index === reached ? "active" : "pending",
  }));
}

function specsOf(status: StatusPayload): string[] {
  const ram = status.spec_ram_mb >= 1024 ? `${+(status.spec_ram_mb / 1024).toFixed(1)} GB memory` : `${status.spec_ram_mb} MB memory`;
  const specs = [status.spec_os, status.spec_arch, `${status.spec_vcpus} CPU`, ram];
  if (status.display_config) specs.push(`${status.display_config.width_px} × ${status.display_config.height_px}`);
  if (status.backend === "mock") specs.push("Simulated backend");
  return specs.filter(Boolean);
}

function seconds(ms: number): string {
  return `${+(ms / 1000).toFixed(1)} s`;
}

/** Only what Core reports; nothing about network or uptime, which it doesn't. */
function factsOf(status: StatusPayload, up: boolean): ComputerFact[] {
  const facts: ComputerFact[] = [];
  const system = [status.spec_os, status.spec_arch].filter(Boolean).join(" · ");
  if (system) facts.push({ label: "System", value: system });
  facts.push({ label: "Isolation", value: "Separate virtual machine" });
  if (status.backend === "mock") facts.push({ label: "Backend", value: "Simulated" });
  if (up && status.guest_ready_ms !== null) facts.push({ label: "Ready in", value: seconds(status.guest_ready_ms) });
  return facts;
}

const STOP: ComputerAction = { command: "stop", label: "Stop computer" };
const PAUSE: ComputerAction = { command: "pause", label: "Pause" };

/**
 * How developers get the sealed image onto this Mac. Dev builds only: the
 * product never tells people to run repository scripts, and production
 * builds drop this text (checked by src/dev/prodBundle.test.ts).
 */
const IMAGE_HOW = "with scripts/build-guest-image (see docs/PROJECT_STATE.md).";

/** One line of real progress for the image setup. */
export function setupProgress(setup: ImageSetup): string {
  const pct = setup.total > 0 ? Math.min(100, Math.floor((setup.done * 100) / setup.total)) : 0;
  switch (setup.stage) {
    case "downloading": return setup.total > 0 ? `Downloading… ${pct}% of ${formatBytes(setup.total)}` : "Downloading…";
    case "verifying": return `Checking the download… ${pct}%`;
    case "unpacking": return `Unpacking… ${pct}%`;
    case "finalizing": return "Finishing…";
    default: return "Starting…";
  }
}

export function computerModel({ connected, native, status, events }: ComputerInputs): ComputerModel {
  const base = { secondary: [] as ComputerAction[], owner: "none" as const, running: false, transitioning: false, steps: [] as BootStep[], specs: [] as string[], facts: [] as ComputerFact[] };
  if (!connected || !status) {
    return { ...base, phase: "unavailable", chip: "Offline", headline: "Computer unavailable",
      body: native ? "Waiting for Pegoles Core to connect." : "Open the Pegoles app to use its computer." };
  }
  const specs = specsOf(status);
  const up = status.computer_state === "running" || status.computer_state === "paused";
  const facts = factsOf(status, up);
  const owner = status.control_owner;
  if (status.viewport_state === "error" || status.computer_state === "error") {
    return { ...base, specs, facts, owner, phase: "error", chip: "Can’t start", headline: "Computer couldn’t start.",
      body: "Its isolated computer ran into a problem. Details has what engineers need.",
      primary: { command: "start", label: "Try again" } };
  }
  // Without the sealed Pegoles image nothing can run: say so plainly, and
  // offer only what can fix it (setting it up, when this build can).
  if (status.backend === "real" && !status.computer_created && status.image_status !== "ready") {
    const setup = status.image_setup;
    if (setup?.installing) {
      return { ...base, specs, facts, phase: "needs-setup", chip: "Setting up", transitioning: true,
        headline: "Setting up Pegoles’ computer…", body: setupProgress(setup),
        primary: { command: "cancel-install", label: "Cancel" } };
    }
    const incomplete = status.image_status === "invalid";
    const headline = incomplete ? "The Pegoles computer image on this Mac is incomplete." : "The Pegoles computer image isn’t installed on this Mac.";
    if (setup?.available) {
      const size = `It downloads once (${formatBytes(setup.download_bytes)}), is checked before use and needs about ${formatBytes(setup.disk_bytes)} of disk space.`;
      return { ...base, specs, facts, phase: "needs-setup", chip: incomplete ? "Image incomplete" : "Not installed", headline,
        body: setup.error ? `${setup.error} ${size}` : size,
        primary: { command: "install", label: incomplete ? "Repair computer" : "Set up computer" } };
    }
    return { ...base, specs, facts, phase: "needs-setup", chip: incomplete ? "Image incomplete" : "Not installed", headline,
      body: "Without it, Pegoles can’t create its isolated computer or work on tasks.",
      devNote: import.meta.env.DEV ? `${incomplete ? "Rebuild" : "Build"} it ${IMAGE_HOW}` : undefined };
  }
  if (!status.computer_created || status.computer_state === "stopped" || status.computer_state === null) {
    return { ...base, specs, facts, phase: "off", chip: "Off", headline: "Off",
      body: "Pegoles will start its isolated computer when it needs it.",
      primary: { command: "start", label: "Start computer" } };
  }
  if (status.computer_state === "stopping") {
    return { ...base, specs, facts, phase: "stopping", chip: "Stopping", transitioning: true, headline: "Shutting down" };
  }
  if (status.computer_state === "paused") {
    return { ...base, specs, facts, owner, phase: "paused", chip: "Paused", headline: "Paused",
      body: "Its apps and files are kept exactly as they were.", primary: { command: "resume", label: "Resume" }, secondary: [STOP] };
  }
  const booting = status.computer_state === "starting" || ["preparing", "starting", "guest_connecting", "display_starting"].includes(status.viewport_state);
  if (booting) {
    return { ...base, specs, facts, phase: "starting", chip: "Starting", transitioning: true, headline: "Starting…",
      steps: bootSteps(status, events), secondary: [STOP] };
  }
  const running = { ...base, specs, facts, owner, running: true, secondary: [PAUSE, STOP] };
  if (owner === "user" || status.viewport_state === "user_controlled") {
    return { ...running, owner: "user", phase: "user", chip: "You have control", headline: "You have control",
      primary: { command: "return", label: "Give control to Pegoles" } };
  }
  if (owner === "agent" || status.viewport_state === "agent_active") {
    return { ...running, owner: "agent", phase: "agent", chip: "Active", headline: "Pegoles has control",
      primary: status.display_attached ? { command: "take", label: "Take control" } : undefined };
  }
  return { ...running, phase: "ready", chip: "Ready", headline: "Ready",
    primary: status.display_attached ? { command: "take", label: "Take control" } : undefined };
}

/**
 * A failed command (e.g. a missing VM helper) is shown where the computer
 * is, not in a global banner: the chip and preview say it couldn't start.
 */
export function withCommandError(model: ComputerModel, failed: boolean): ComputerModel {
  if (!failed || model.running || model.phase === "unavailable") return model;
  return { ...model, phase: "error", chip: "Can’t start", transitioning: false, steps: [],
    primary: model.primary ?? { command: "start", label: "Try again" } };
}

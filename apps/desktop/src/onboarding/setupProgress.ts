import type { InstallErrorKind, StatusPayload } from "../lib/tauri";
import type { LocalView } from "../state/localModel";

/**
 * "Set up Pegoles" as one job made of Core's real jobs: the AI model, the
 * computer image, then the computer's first start. Pure: every number is
 * a byte count Core reported; nothing is timed or invented.
 */
export type SetupStepId = "model" | "computer" | "verify" | "finish";

export const SETUP_STEPS: readonly { readonly id: SetupStepId; readonly label: string }[] = [
  { id: "model", label: "Downloading AI model" },
  { id: "computer", label: "Preparing your computer" },
  { id: "verify", label: "Verifying files" },
  { id: "finish", label: "Almost ready" },
];

/** Which parts this setup covers (fixed when it starts, so totals never jump). */
export interface SetupPlan {
  readonly model: boolean;
  readonly image: boolean;
}

export type SetupPhase = "idle" | "working" | "paused" | "failed" | "done";

export interface SetupProgress {
  readonly phase: SetupPhase;
  /** What is happening right now, in words ("Preparing Pegoles" before bytes flow). */
  readonly headline: string;
  readonly steps: readonly { readonly id: SetupStepId; readonly label: string; readonly state: "done" | "active" | "todo" }[];
  readonly doneBytes: number;
  readonly totalBytes: number;
  /** Whole percent of real bytes, or null while nothing is measured. */
  readonly percent: number | null;
  /** Bytes are flowing (speed and time left make sense). */
  readonly transferring: boolean;
  /** Parts of the download are already on disk. */
  readonly resumable: boolean;
  readonly error: { readonly kind: InstallErrorKind; readonly technical: string } | null;
}

export interface SetupInputs {
  readonly plan: SetupPlan;
  readonly local: LocalView;
  readonly status: StatusPayload | null;
  /** The person pressed "Set up Pegoles" (and hasn't paused). */
  readonly running: boolean;
  /** A pause the person asked for. */
  readonly paused: boolean;
  /** A setup command Core refused, or the computer's first start failing (Core's words). */
  readonly commandError: string | null;
}

export function modelInstalled(local: LocalView): boolean {
  return local.stage === "ready" || local.stage === "downloaded";
}

export function imageReady(status: StatusPayload | null): boolean {
  return status?.image_status === "ready";
}

export function computerReady(status: StatusPayload | null): boolean {
  return status?.computer_state === "running" && status.guest_state === "ready";
}

/** Plan from what is missing right now. */
export function planFor(local: LocalView, status: StatusPayload | null): SetupPlan {
  return { model: !modelInstalled(local), image: !imageReady(status) };
}

/** Image stages as Core names them → our words. */
function imageKind(error: string): InstallErrorKind {
  if (/space|ENOSPC|disk full/i.test(error)) return "disk_space";
  if (/network|connect|timed? ?out|dns|resolve|http|tls|offline/i.test(error)) return "network";
  if (/checksum|sha|digest|verif|corrupt|mismatch/i.test(error)) return "corrupted";
  return "other";
}

export function setupProgress(inputs: SetupInputs): SetupProgress {
  const { plan, local, status, running, paused, commandError } = inputs;
  const modelDone = !plan.model || modelInstalled(local);
  const imageDone = !plan.image || imageReady(status);
  const setup = status?.image_setup;
  const modelTotal = plan.model ? local.totalBytes ?? local.model?.size_bytes ?? 0 : 0;
  const imageTotal = plan.image ? setup?.download_bytes ?? 0 : 0;

  const modelBytes = !plan.model ? 0 : modelDone ? modelTotal
    : local.stage === "preparing" || local.stage === "paused" || local.stage === "failed" ? Math.min(local.doneBytes ?? 0, modelTotal) : 0;
  const imageDownloading = !!setup?.installing && setup.stage === "downloading";
  const imageBytes = !plan.image ? 0 : imageDone ? imageTotal
    : imageDownloading ? Math.min(setup.done, imageTotal)
      : setup?.installing ? imageTotal : 0;
  const doneBytes = modelBytes + imageBytes;
  const totalBytes = modelTotal + imageTotal;
  const percent = totalBytes > 0 ? Math.max(0, Math.min(100, Math.floor((doneBytes / totalBytes) * 100))) : null;

  const modelVerifying = local.stage === "preparing" && local.step !== "downloading";
  const imageVerifying = !!setup?.installing && setup.stage === "verifying";
  const booting = modelDone && imageDone && !computerReady(status);
  const allDone = modelDone && imageDone && computerReady(status);

  let active: SetupStepId | null = null;
  let headline = "Preparing Pegoles";
  if (allDone) {
    headline = "Pegoles is set up";
  } else if (!modelDone && local.stage === "preparing") {
    active = modelVerifying ? "verify" : "model";
    headline = modelVerifying ? "Verifying files" : "Downloading AI model";
  } else if (modelDone && !imageDone && setup?.installing) {
    active = imageVerifying ? "verify" : "computer";
    headline = imageVerifying ? "Verifying files" : "Preparing your computer";
  } else if (booting && running) {
    active = "finish";
    headline = "Almost ready";
  }

  const doneOf = (id: SetupStepId): boolean => {
    switch (id) {
      case "model": return modelDone;
      case "computer": return imageDone;
      case "verify": return modelDone && imageDone;
      case "finish": return allDone;
    }
  };
  const steps = SETUP_STEPS
    .filter((step) => (step.id === "model" ? plan.model : step.id === "computer" ? plan.image : true))
    .map((step) => ({ ...step, state: doneOf(step.id) ? "done" as const : step.id === active ? "active" as const : "todo" as const }));

  let error: SetupProgress["error"] = null;
  if (local.stage === "failed" && plan.model && !modelDone) {
    error = { kind: local.errorKind ?? "other", technical: local.error ?? "The model setup didn't finish." };
  } else if (setup?.error && !setup.installing && plan.image && !imageDone && !/cancel/i.test(setup.error)) {
    error = { kind: imageKind(setup.error), technical: setup.error };
  } else if (commandError) {
    error = { kind: imageKind(commandError), technical: commandError };
  } else if (modelDone && imageDone && (status?.guest_state === "error" || status?.guest_state === "incompatible" || status?.computer_state === "error")) {
    error = { kind: "other", technical: `computer ${status.computer_state ?? "not created"}, guest runtime ${status.guest_state}` };
  }

  const busy = local.stage === "preparing" || !!setup?.installing;
  const phase: SetupPhase = allDone ? "done"
    : error && !busy ? "failed"
      : paused && !busy ? "paused"
        : running || busy ? "working"
          : "idle";
  return {
    phase,
    headline: phase === "paused" ? "Paused" : phase === "failed" ? "Setup stopped" : headline,
    steps,
    doneBytes,
    totalBytes,
    percent,
    transferring: (local.stage === "preparing" && local.step === "downloading") || imageDownloading,
    resumable: doneBytes > 0 && doneBytes < totalBytes,
    error,
  };
}

/** What "Try again" can promise, in plain words. */
export function errorHint(kind: InstallErrorKind): string {
  switch (kind) {
    case "disk_space": return "There isn't enough free disk space. Free up some space, then try again.";
    case "network": return "Pegoles couldn't download what it needs. Check your internet connection, then try again. What was already downloaded is kept.";
    case "corrupted": return "A downloaded file didn't pass its safety check, so Pegoles will download it again.";
    case "other": return "Try again. If it keeps happening, open Technical details.";
  }
}

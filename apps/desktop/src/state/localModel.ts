import type { InstallErrorKind, Intelligence, LocalModelInfo, ModelInstallStatus } from "../lib/tauri";
import { formatBytes } from "../lib/format";

/**
 * Pegoles Local as people see it: one stage for the chosen model, derived
 * only from what Core reports (its catalog, the model store, the latest
 * setup job and the runtime). Pure, so Settings, the task notice and the
 * composer all say the same thing.
 */
export type LocalStage =
  /** No answer from Core yet. */
  | "checking"
  /** This computer can't run it (e.g. an Intel Mac). */
  | "unsupported"
  | "not-set-up"
  /** Downloading, verifying or finishing. */
  | "preparing"
  /** Part of the download is on disk; a retry resumes from there. */
  | "paused"
  | "failed"
  /** Installed files didn't pass the store's check. */
  | "damaged"
  /** Installed, but the runtime isn't ready on this Mac. */
  | "downloaded"
  | "ready";

export type PreparingStep = "downloading" | "verifying" | "finishing";

export interface LocalView {
  readonly stage: LocalStage;
  /** The chosen model (catalog entry), when Core has answered. */
  readonly model: LocalModelInfo | null;
  readonly step?: PreparingStep;
  readonly doneBytes?: number;
  readonly totalBytes?: number;
  /** Core's plain-language reason a setup failed. */
  readonly error?: string;
  readonly errorKind?: InstallErrorKind | null;
  /** Why installed files were rejected (damaged). */
  readonly reason?: string;
  /** Why the runtime can't run it on this Mac, in Core's words. */
  readonly problem: string | null;
  /** Loaded in the worker right now. */
  readonly running: boolean;
  /** Measured memory of the worker, only while running. */
  readonly footprintBytes: number | null;
}

const ACTIVE = new Set<ModelInstallStatus["phase"]>(["downloading", "verifying", "finalizing"]);

export const STEP_LABEL: Record<PreparingStep, string> = {
  downloading: "Downloading model…",
  verifying: "Verifying…",
  finishing: "Finishing…",
};

/** A setup job is in flight (downloading, verifying or finishing). */
export function installActive(status: ModelInstallStatus | null | undefined): boolean {
  return !!status && ACTIVE.has(status.phase);
}

/** Core's words as one sentence: capitalised, ending in a full stop. */
export function sentence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return trimmed;
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`;
}

/** The runtime's reason without the error's type prefix ("local model runtime is not installed: …"). */
function plainProblem(problem: string | null): string | null {
  if (!problem?.trim()) return null;
  return sentence(problem.replace(/^local model runtime is not installed:\s*/i, ""));
}

/**
 * The store checks each file as it lands, so "verifying" also flashes
 * between files: it only reads as verifying once every byte is in.
 */
function stepOf(job: ModelInstallStatus): PreparingStep {
  if (job.phase === "finalizing") return "finishing";
  if (job.phase === "verifying" && job.total_bytes > 0 && job.done_bytes >= job.total_bytes) return "verifying";
  return "downloading";
}

/** The model Pegoles Local uses: the chosen one, else the catalog default. */
export function chosenModel(intelligence: Intelligence): LocalModelInfo | null {
  const { models, default_model } = intelligence.local;
  return models.find((model) => model.id === intelligence.local_model) ?? models.find((model) => model.id === default_model) ?? models[0] ?? null;
}

export function localView(intelligence: Intelligence | null): LocalView {
  const idle = { problem: null, running: false, footprintBytes: null } as const;
  if (!intelligence) return { ...idle, stage: "checking", model: null };
  const { local } = intelligence;
  const model = chosenModel(intelligence);
  const base = { ...idle, model, problem: local.runtime_ready ? null : plainProblem(local.runtime_problem) };
  if (!model) return { ...base, stage: "unsupported" };

  const job = local.install?.model === model.id ? local.install : null;
  if (job && ACTIVE.has(job.phase)) {
    return { ...base, stage: "preparing", step: stepOf(job), doneBytes: job.done_bytes, totalBytes: job.total_bytes || model.size_bytes };
  }
  if (model.state === "installed" || job?.phase === "ready") {
    if (!local.runtime_ready) return { ...base, stage: local.host_supported ? "downloaded" : "unsupported" };
    const running = local.loaded_model === model.id;
    return { ...base, stage: "ready", running, footprintBytes: running ? local.worker_footprint_bytes : null };
  }
  if (!local.host_supported) return { ...base, stage: "unsupported" };
  if (model.state === "invalid") return { ...base, stage: "damaged", reason: model.invalid_reason ?? undefined };
  const partial = model.state === "partial" ? model.partial_bytes ?? 0 : 0;
  if (job?.phase === "failed") {
    return {
      ...base, stage: "failed", error: job.error ?? "The setup didn’t finish.", errorKind: job.error_kind,
      doneBytes: partial || undefined, totalBytes: model.size_bytes,
    };
  }
  if (partial > 0) return { ...base, stage: "paused", doneBytes: partial, totalBytes: model.size_bytes };
  return { ...base, stage: "not-set-up" };
}

/** "940 MB of 2.2 GB". */
export function progressText(done: number, total: number): string {
  return `${formatBytes(done)} of ${formatBytes(total)}`;
}

/** Whole percent of a real measure, or null when there is none. */
export function percentOf(done: number | undefined, total: number | undefined): number | null {
  if (done === undefined || !total || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.floor((done / total) * 100)));
}

/** What the button that (re)starts the setup says in each stage. */
export function setupAction(view: LocalView): string {
  switch (view.stage) {
    case "paused": return "Resume";
    case "damaged": return "Set up again";
    case "failed":
      if (view.errorKind === "corrupted") return "Download again";
      return view.errorKind === "network" && view.doneBytes ? "Resume" : "Try again";
    default: return "Set up";
  }
}

/** Setup can be (re)started from here. */
export function canSetUp(view: LocalView): boolean {
  return view.stage === "not-set-up" || view.stage === "paused" || view.stage === "failed" || view.stage === "damaged";
}

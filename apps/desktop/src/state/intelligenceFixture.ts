/**
 * Test fixtures only: Intelligence payloads shaped exactly like Core's.
 * Never imported by production code.
 */
import type { Intelligence, LocalModelInfo, LocalRuntime, ModelInstallStatus, ModelSettings } from "../lib/tauri";

export const MAI: LocalModelInfo = {
  id: "mai-ui-2b-6bit", display_name: "MAI-UI 2B", family: "MaiUi", parameters: "2B", quantization: "6bit",
  size_bytes: 2_226_454_187, license: "apache-2.0", source: "huggingface.co/mlx-community/MAI-UI-2B-6bit-v2 @ cb57cf2fc99f",
  state: "not_installed", partial_bytes: null, invalid_reason: null, downloadable: true, recommended_min_ram_gb: null,
};

export const QWEN: LocalModelInfo = {
  ...MAI, id: "qwen3-vl-2b-4bit", display_name: "Qwen3-VL 2B Instruct", family: "Qwen3Vl", quantization: "4bit",
  size_bytes: 1_798_021_605, source: "huggingface.co/mlx-community/Qwen3-VL-2B-Instruct-4bit @ 9c4f5209e57b",
};

export const NO_KEY: ModelSettings = {
  configured: false, key_source: null, model: "claude-opus-5", effort: "high",
  models: ["claude-opus-5", "claude-sonnet-5", "claude-opus-5-5"], efforts: ["low", "medium", "high", "xhigh", "max"],
};

export interface IntelligencePatch {
  readonly provider?: Intelligence["provider"];
  readonly local_model?: string;
  /** Applied to the chosen (first) model. */
  readonly model?: Partial<LocalModelInfo>;
  readonly models?: readonly LocalModelInfo[];
  readonly local?: Partial<LocalRuntime>;
  readonly anthropic?: Partial<ModelSettings>;
}

export function intelligenceOf(patch: IntelligencePatch = {}): Intelligence {
  const models = patch.models ?? [{ ...MAI, ...patch.model }];
  return {
    provider: patch.provider ?? "local",
    local_model: patch.local_model ?? models[0].id,
    anthropic: { ...NO_KEY, ...patch.anthropic },
    local: {
      runtime_ready: true, runtime_problem: null, loaded_model: null, worker_footprint_bytes: null,
      default_model: MAI.id, models, install: null, chip: "Apple M3 Pro", memory_bytes: 18 * 2 ** 30, apple_silicon: true, host_supported: true,
      ...patch.local,
    },
  };
}

export function installOf(phase: ModelInstallStatus["phase"], done: number, extra: Partial<ModelInstallStatus> = {}): ModelInstallStatus {
  return { model: MAI.id, phase, done_bytes: done, total_bytes: MAI.size_bytes, error: null, error_kind: null, ...extra };
}

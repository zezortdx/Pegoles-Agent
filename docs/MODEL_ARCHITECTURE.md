# Model architecture

How Pegoles decides what to do on its computer, which models it can use,
and where each piece runs. Security properties: `docs/SECURITY.md`.
Measured numbers: `docs/PERFORMANCE.md` and
`benchmarks/local-models/results.json`.

## Principle

Pegoles works without any API key. The default planner, **Pegoles
Local**, is a vision-language model that runs on the user's Mac (the
host). Cloud providers are optional. Every provider — local or cloud —
is an untrusted planner: it only proposes actions, and the same
deterministic path decides and executes them.

```text
HOST (macOS, Apple silicon)
  Pegoles desktop (Tauri)                    webview: renders, never plans
    agent::start_run ── settings.provider ─┬─ LocalPlanner ──▶ InferenceBackend
                                           │     (prompt, strict parse)   │ JSONL v1, pipes
                                           │                              ▼
                                           │                 MLX worker (Python, persistent,
                                           │                 sandbox-exec: no network,
                                           │                 cleared env) ─▶ mlx-vlm ─▶ Metal
                                           └─ AnthropicPlanner ─HTTPS─▶ Anthropic API (optional)
    runner (budgets, loop/batch brakes, cancellation)
      │ typed ComputerAction only
      ▼
    Core executor ─▶ Pegoles Policy (exhaustive, deterministic) ─▶ control arbitration
      │
      ▼ vsock (authenticated reserved port)
GUEST VM (no network): Debian + Weston + foot + guest runtime
```

The model never runs in the guest and never talks to it. It never
receives a host capability: its only output is text, which becomes at
most one typed `ComputerAction` per step.

## Layers

| Layer | Crate / file | Job |
|---|---|---|
| Planner contract | `pegoles-agent/src/planner.rs` | `Planner::{start, next}` → `PlannerTurn::{Calls, Done, Failed}`; steps are `ComputerAction`, `Observe`, `Reply` |
| Runner | `pegoles-agent/src/runner.rs` | turn/action/time budgets, failure brake, batch halt, cancellation between steps |
| Local provider | `pegoles-agent/src/local/` | per-family prompts (`prompt.rs`), strict parser (`parse.rs`), context window, loop brake, retries, crash recovery (`mod.rs`) |
| Cloud provider | `pegoles-agent/src/anthropic.rs` | Messages API computer-use toolset, raw HTTPS; key from Keychain |
| Inference boundary | `pegoles-inference/src/backend.rs` | `InferenceBackend`: chat + images → text, load/unload/memory; knows nothing about actions |
| MLX backend | `pegoles-inference/src/worker.rs` + `workers/mlx/pegoles_mlx_worker.py` | supervised persistent worker process |
| Model store | `pegoles-inference/src/{catalog,store,download}.rs` | pinned catalog, resumable download, SHA-256 verification, atomic install, remove |
| Hardware | `pegoles-inference/src/hardware.rs` | chip, RAM, cores, system memory in use, pressure, swap, per-process footprint |
| App wiring | `apps/desktop/src-tauri/src/{agent,local,commands}.rs` | provider choice, install progress events, shared worker, idle unload |

Adding a provider means implementing `Planner` (cloud APIs) or
`InferenceBackend` (another local runtime: llama.cpp, CUDA, Vulkan).
Nothing downstream of the planner changes.

### Provider selection

`settings.json` (never secrets): `{"provider": "local" | "anthropic",
"local_model": "<catalog id>", "anthropic": {"model", "effort"}}`.
Default: `local`. A settings file from before this phase
(`{"model","effort"}`) keeps the Anthropic choices but does not switch
the user away from Local. OpenAI / Gemini / OpenAI-compatible providers
are not implemented; they slot in as `Planner`s (cloud APIs) or as an
`InferenceBackend` speaking `/v1/chat/completions` (a self-hosted
VLM reusing the local prompts and parser).

### Hybrid (prepared, not built)

A hybrid planner would be one more `Planner` that owns a local planner
and a cloud planner and delegates per turn (e.g. cloud for high-level
planning after repeated failures, local for grounding). It needs no
change to the runner or policy. It must not be automatic: consulting
the cloud changes privacy and cost, so it requires explicit user
consent in Settings.

## Local provider

**One action per step.** Each turn the model sees: the objective (the
only authoritative text), the last `history_steps` (6) steps as the
model's own tool calls in its trained format with their results, the
count of older steps, optional earlier screenshots (`history_images`,
default 0), and the current screen. Security state (policy, budgets)
is never in model context.

**Execution.** A parsed action becomes one planned call:
`[action, Wait(settle 400 ms), Observe]`. If the action fails (policy
block, interruption) the batch halts and the next turn only observes.

**Strict parsing** (`local/parse.rs`): exactly one closed
`<tool_call>{json}</tool_call>`; the tool name must be the family's
tool (a missing name is accepted, a different one is not); only
`name`/`arguments` at the top level; per-action allow-listed argument
keys; coordinates finite and within the family scale (MAI-UI 0–999,
Qwen3-VL 0–1000, a 4-number box collapses to its center); text ≤
`MAX_TYPE_CHARS`, no control characters except `\n`/`\t`, no bidi or
zero-width characters; 1–4 known key names; bounded scroll and wait.
Everything else is an error string the model reads on retry — never an
action.

**Retries and brakes (deterministic).** An invalid reply is retried up
to twice, the first retry at temperature 0.4, the second at 0.8 (at 0
the same prompt repeats the mistake). The loop brake remembers the last
12 (action, screen) pairs: re-proposing an action already taken on the
same screen draws a warning, the third repeat stops the task (this also
catches A-B-A-B toggling). The runner's turn, action, time and
failed-turn budgets apply on top.

**Crash recovery.** A crashed, hung (timeout) or protocol-violating
worker is killed; the planner respawns it, reloads the verified model
and retries the inference once before failing the task with a clear
reason. Cancellation is cooperative (the worker stops between tokens)
with a forced kill after 3 s.

### Model formats

| Family | Tool | Format | Coordinates |
|---|---|---|---|
| MAI-UI (Qwen3-VL fine-tune) | `mobile_use` | its trained prompt: `<thinking>…</thinking><tool_call>{…}</tool_call>`; actions click, double_click, right_click, type, key, system_button (enter/back), swipe (= scroll, touch direction), drag, wait, terminate, answer | 0–999 of the image |
| Qwen3-VL Instruct | `computer_use` | official Qwen-Agent function schema; key, type, mouse_move, left/right/middle/double_click, left_click_drag, scroll, hscroll, wait, terminate, answer | 0–1000 of the image |

Both families see identical information at identical resolution.

### Observation pipeline

The guest framebuffer (1440×900) is captured once per step; the user
preview and the model input are independent. The planner chooses the
model input size (`observe_long_side`, never upscaled, both sides on
the 32-px vision grid: 1440×896 native, 1024×640 reduced) and the
worker resizes (Lanczos). Coordinates are relative, so they map back
without knowing the resize. Default: native 1440×896 (best measured
grounding; 1024×640 is ~0.5 s faster per step at a noise-level accuracy
cost). A crop/zoom second pass is supported by the protocol
(`image_prep.crop`) but not used: when the default model clicks, it
hits (9/10 first clicks, median error 5 px); its failures are agentic
habits and 13-px targets, not coarse grounding.

## MLX worker

`workers/mlx/pegoles_mlx_worker.py`, protocol version 1, one JSON
object per line:

| op | effect |
|---|---|
| `hello` | versions, Metal availability |
| `load {model_dir}` | load once (absolute path to a verified store dir); `trust_remote_code=False`, transformers' dynamic-module loader replaced by a refusal |
| `generate {messages, images, image_prep, max_tokens, temperature}` | bounded chat (≤16 messages, ≤64 KiB text, ≤2 PNG images ≤16 MiB, ≤4096 px, max_tokens ≤2048); streams internally, replies once with text (≤32 K chars), token counts, timings, memory |
| `cancel {target}` | handled out of band; stops generation between tokens |
| `stats`, `unload`, `shutdown` | memory report, free the model, exit |

Supervisor (`worker.rs`): spawned as `/usr/bin/sandbox-exec -p <no
network, no writes to user/system locations> python -I worker.py` with
a cleared environment (no API keys, offline flags), cwd `/`, MLX
buffer cache capped at 256 MiB (peak footprint −0.9 GB, measured); replies
bounded to 1 MiB per line (oversized or invalid → kill); stderr kept as
a 40-line ring for diagnostics; per-request timeouts (load 240 s,
generate 120 s); respawn on next use.

The app keeps one worker for all runs (model stays loaded between
tasks), verifies the chosen model's bytes in the background at startup,
and stops the worker after 10 minutes unused (`local.rs::IDLE_UNLOAD`).

## Model store

`~/Library/Application Support/Pegoles/models/` (never in Git):

```text
models/<id>/                exactly the catalog files + pegoles-model.json
models/.staging/<id>/       download in progress (*.part resume by HTTP range)
models/.trash/              replaced / removed, deleted best-effort
```

The catalog (`crates/pegoles-inference/catalog/models.json`) is compiled
into the app: id, family, architecture, quantization, source (repository
+ full commit hash, or a documented local conversion), license, minimum
Pegoles version, recommended RAM (only once measured), and every file's
size and SHA-256. Rules: relative, non-hidden paths only; only
`.safetensors/.json/.txt/.jinja/.model/.tiktoken` (no pickles, no
Python); a `config.json` with `auto_map` (remote code) is refused.

Install = free-space check (+2 GB headroom) → download in 64 MiB ranged
requests (each time-bounded, HTTPS only, redirects ≤ 8) → SHA-256 of the
whole file (including resumed bytes) → rename into staging → manifest
written and synced → staging renamed into place. A mismatch deletes the
partial file (no resume of corrupt bytes). Loading re-verifies every
byte (once per app session; ~4 s for 2.2 GB) and re-checks the layout
(no extra files, no symlinks, sizes). Remove moves the directory to
`.trash` first. Upgrade = install the new id, then remove the old.

Tools: `cargo run -p pegoles-inference --example models -- list|install|verify|import|remove|hw`,
`scripts/local-model/pin_hf_model.py` (catalog entry from a pinned HF
revision), `scripts/local-model/pin_local_model.py` (from a local
conversion).

## Runtime (Python + MLX)

Shipped inside the app at `Contents/Resources/runtime/python`, built by
`scripts/local-model/build-runtime.sh`: python-build-standalone CPython
3.12.14 (release 20260924, pinned by SHA-256) plus exactly the 33 wheels in
`workers/mlx/requirements.lock` (every file pinned by SHA-256, wheels only,
`--no-deps`; the server, audio, OpenCV, SciPy and CLI extras of mlx-vlm
are not shipped because the worker never imports them). pip, setuptools,
console scripts, the test suite and Tk are removed; bytecode is
precompiled with hash-checked `.pyc`, so two builds produce the same tree
digest (`runtime-manifest.json`). The app never runs pip, never resolves
dependencies and never downloads Python code. Release builds run only
this interpreter; debug builds also find `target/pegoles-runtime`.
Licenses of everything shipped: `THIRD_PARTY_NOTICES.md` in the bundle
(`scripts/release/third-party-notices.sh`).

## Model choice

Default: **MAI-UI-2B, 6-bit** (`mlx-community/MAI-UI-2B-6bit-v2`,
Apache-2.0, 2.23 GB). Offered alternative: Qwen3-VL-2B-Instruct 6-bit.
In the catalog but not offered: both 4-bit variants (they failed the
benchmark). Chosen on the Pegoles benchmark (22 real-VM tasks): MAI-UI
achieved 17/22 goals (16 clean) vs 11/22 (4 clean) for Qwen3-VL at the
same size, memory and latency. Data and decision matrix:
`benchmarks/local-models/README.md`, `results.json`.

Hardware: only an M4 Pro with 24 GB was available. Measured Pegoles
footprint at peak is ≈4.5–5 GB (model worker ~4 GB with the cache cap,
VM ~0.55 GB, runtime ~0.1 GB) plus the webview; 16 GB Macs are expected
to fit, 8 GB Macs are untested and likely to swap. The catalog's
`recommended_min_ram_gb` stays empty until measured on such machines.

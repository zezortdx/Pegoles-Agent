//! llama.cpp (text model + multimodal projector) behind the worker's ops.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::mtmd::{MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::{list_llama_ggml_backend_devices, LlamaBackendDeviceType};
use serde_json::{json, Value};

use crate::protocol::{MAX_TEXT_CHARS, MEDIA_MARKER};

/// Context: one screenshot (≤ ~1600 image tokens at the planner's sizes),
/// the prompt and history, and the reply.
const N_CTX: u32 = 8192;
const N_BATCH: u32 = 2048;
/// llama.cpp warns that Qwen3-VL grounding needs at least this many image
/// tokens; smaller screenshots are scaled up to it.
const IMAGE_MIN_TOKENS: i32 = 1024;
/// A file name inside the model folder: no separators, no parent steps.
pub fn plain_gguf_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.ends_with(".gguf")
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub struct Accelerator {
    pub backend: String,
    pub device: String,
    pub gpu: bool,
}

/// The best device llama.cpp found (a GPU if any, else the CPU).
/// What llama.cpp will run on: the first GPU it initialized, unless the
/// CPU is forced (or there is none).
pub fn accelerator(force_cpu: bool) -> Accelerator {
    let devices = list_llama_ggml_backend_devices();
    let gpu = devices.iter().filter(|_| !force_cpu).find(|d| {
        matches!(
            d.device_type,
            LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
        )
    });
    let cpu = devices
        .iter()
        .find(|d| matches!(d.device_type, LlamaBackendDeviceType::Cpu));
    match gpu.or(cpu) {
        Some(d) => Accelerator {
            backend: backend_label(&d.backend),
            device: d.description.chars().take(80).collect(),
            gpu: gpu.is_some(),
        },
        None => Accelerator {
            backend: "CPU".into(),
            device: String::new(),
            gpu: false,
        },
    }
}

/// ggml's registry names as people know them ("MTL" is Metal).
pub fn backend_label(name: &str) -> String {
    match name.to_ascii_uppercase().as_str() {
        "MTL" | "METAL" => "Metal".into(),
        "VULKAN" => "Vulkan".into(),
        "CUDA" => "CUDA".into(),
        "CPU" => "CPU".into(),
        _ => name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(20)
            .collect(),
    }
}

fn threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 16) as i32)
        .unwrap_or(4)
}

/// A loaded model: the context borrows the model, so both live together
/// and are dropped context-first (see `Drop`).
pub struct Loaded {
    ctx: std::mem::ManuallyDrop<LlamaContext<'static>>,
    mtmd: MtmdContext,
    model: Box<LlamaModel>,
}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: the context borrows `model`; it is dropped first and never
        // used again. `model` is boxed, so its address never moved.
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ctx) };
    }
}

pub fn load(
    backend: &LlamaBackend,
    dir: &str,
    model_file: &str,
    projector_file: &str,
    gpu: bool,
) -> Result<Loaded, String> {
    if !plain_gguf_name(model_file) || !plain_gguf_name(projector_file) {
        return Err("model and projector must be plain .gguf file names".into());
    }
    let model_path = Path::new(dir).join(model_file);
    let projector = Path::new(dir).join(projector_file);
    if !model_path.is_file() || !projector.is_file() {
        return Err("the model folder is missing its GGUF files".into());
    }
    let params = LlamaModelParams::default().with_n_gpu_layers(if gpu { 999 } else { 0 });
    let model = Box::new(
        LlamaModel::load_from_file(backend, &model_path, &params)
            .map_err(|e| format!("model: {e}"))?,
    );
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(N_CTX))
        .with_n_batch(N_BATCH)
        .with_n_ubatch(N_BATCH)
        .with_n_threads(threads())
        .with_n_threads_batch(threads());
    // SAFETY: the context borrows `*model`, which is heap-allocated, never
    // moved and outlives the context (Loaded drops the context first).
    let model_ref: &'static LlamaModel = unsafe { &*(model.as_ref() as *const LlamaModel) };
    let ctx = model_ref
        .new_context(backend, ctx_params)
        .map_err(|e| format!("context: {e}"))?;
    let marker = std::ffi::CString::new(MEDIA_MARKER).expect("no NUL");
    let mtmd_params = MtmdContextParams {
        use_gpu: gpu,
        print_timings: false,
        n_threads: threads(),
        media_marker: marker,
        image_min_tokens: IMAGE_MIN_TOKENS,
        image_max_tokens: -1,
    };
    let projector = projector.to_str().ok_or("the model path is not UTF-8")?;
    let mtmd = MtmdContext::init_from_file(projector, model_ref, &mtmd_params)
        .map_err(|e| format!("projector: {e}"))?;
    if !mtmd.support_vision() {
        return Err("the projector has no vision support".into());
    }
    Ok(Loaded {
        ctx: std::mem::ManuallyDrop::new(ctx),
        mtmd,
        model,
    })
}

pub struct Generated {
    pub text: String,
    pub finish: &'static str,
    pub prompt_tokens: usize,
    pub generation_tokens: usize,
    pub image_ms: f64,
    pub first_token_ms: f64,
    pub generate_ms: f64,
}

pub fn generate(
    loaded: &mut Loaded,
    prompt: &str,
    images: &[image::RgbImage],
    max_tokens: u32,
    temperature: f32,
    cancel: &AtomicBool,
) -> Result<Generated, String> {
    let started = Instant::now();
    loaded.ctx.clear_kv_cache();
    let bitmaps = images
        .iter()
        .map(|img| {
            MtmdBitmap::from_image_data(img.width(), img.height(), img.as_raw())
                .map_err(|e| format!("image: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let refs: Vec<&MtmdBitmap> = bitmaps.iter().collect();
    let chunks = loaded
        .mtmd
        .tokenize(
            MtmdInputText {
                text: prompt.to_string(),
                add_special: true,
                parse_special: true,
            },
            &refs,
        )
        .map_err(|e| format!("tokenize: {e}"))?;
    let prompt_tokens = chunks.total_tokens();
    if prompt_tokens + max_tokens as usize >= N_CTX as usize {
        return Err(format!("prompt too long ({prompt_tokens} tokens)"));
    }
    let prefilled = chunks
        .eval_chunks(&loaded.mtmd, &loaded.ctx, 0, 0, N_BATCH as i32, true)
        .map_err(|e| format!("prefill: {e}"))?;
    let image_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut sampler = if temperature <= 0.0 {
        LlamaSampler::greedy()
    } else {
        LlamaSampler::chain_simple([LlamaSampler::temp(temperature), LlamaSampler::dist(0x5EED)])
    };
    let mut batch = LlamaBatch::new(1, 1);
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut text = String::new();
    let mut first_token_ms = 0.0;
    let mut finish = "length";
    let mut generated = 0usize;
    for n_past in (prefilled..).take(max_tokens as usize) {
        if cancel.load(Ordering::SeqCst) {
            finish = "cancelled";
            break;
        }
        let token = sampler.sample(&loaded.ctx, -1);
        sampler.accept(token);
        if generated == 0 {
            first_token_ms = started.elapsed().as_secs_f64() * 1000.0 - image_ms;
        }
        generated += 1;
        if loaded.model.is_eog_token(token) {
            finish = "stop";
            break;
        }
        let piece = loaded
            .model
            .token_to_piece(token, &mut decoder, false, None)
            .unwrap_or_default();
        text.push_str(&piece);
        if text.chars().count() > MAX_TEXT_CHARS {
            finish = "text_limit";
            break;
        }
        batch.clear();
        batch
            .add(token, n_past, &[0], true)
            .map_err(|e| format!("batch: {e}"))?;
        loaded
            .ctx
            .decode(&mut batch)
            .map_err(|e| format!("decode: {e}"))?;
    }
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    Ok(Generated {
        text,
        finish,
        prompt_tokens,
        generation_tokens: generated,
        image_ms,
        first_token_ms,
        generate_ms: started.elapsed().as_secs_f64() * 1000.0 - image_ms,
    })
}

pub fn memory_fields() -> Value {
    // llama.cpp does not report allocator totals; the host measures the
    // process from the outside.
    json!({"active_bytes": 0, "peak_bytes": 0, "cache_bytes": 0})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_are_the_ones_people_know() {
        assert_eq!(backend_label("MTL"), "Metal");
        assert_eq!(backend_label("Vulkan"), "Vulkan");
        assert_eq!(backend_label("cpu"), "CPU");
        assert_eq!(backend_label("odd<name>\n"), "oddname");
    }
}

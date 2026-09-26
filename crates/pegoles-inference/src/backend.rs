//! The replaceable inference boundary.
//!
//! A backend runs a verified model and turns (chat, images) into text.
//! It knows nothing about Pegoles actions, policy or the VM; the local
//! planner owns prompting and parsing. MLX (a persistent Python worker)
//! is the Apple Silicon reference implementation; llama.cpp / CUDA /
//! Vulkan backends would implement the same trait.

use serde::Serialize;

use crate::store::VerifiedModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// Index into `GenerateRequest::images`.
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl ChatMessage {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![Part::Text(text.into())],
        }
    }
}

/// A PNG plus how the backend should prepare it for the model: crop in
/// source pixels, then resize to exact model-input pixels. The planner
/// chooses both, so it always knows how model coordinates map back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInput {
    pub png: std::sync::Arc<Vec<u8>>,
    pub crop: Option<[u32; 4]>,
    pub resize: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenerateRequest {
    pub messages: Vec<ChatMessage>,
    pub images: Vec<ImageInput>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub timeout: std::time::Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Timings {
    pub image_ms: f64,
    pub first_token_ms: f64,
    pub generate_ms: f64,
    /// Backend-side total (excludes transport).
    pub total_ms: f64,
    /// Round trip as the caller saw it (includes transport/encoding).
    pub wall_ms: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BackendMemory {
    /// Framework allocations (e.g. MLX active memory).
    pub active_bytes: u64,
    pub peak_bytes: u64,
    pub cache_bytes: u64,
    /// Whole-process footprint as the OS accounts it.
    pub process_footprint_bytes: Option<u64>,
    pub process_peak_footprint_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GenerateResponse {
    pub text: String,
    /// "stop", "length", "cancelled", "text_limit".
    pub finish: String,
    pub prompt_tokens: u32,
    pub generation_tokens: u32,
    pub prompt_tps: f64,
    pub generation_tps: f64,
    pub image_sizes: Vec<(u32, u32)>,
    pub timings: Timings,
    pub memory: BackendMemory,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LoadReport {
    pub model_id: String,
    pub load_ms: f64,
    pub verify_ms: u64,
    pub memory: BackendMemory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackendInfo {
    pub backend: String,
    pub runtime_version: String,
    pub accelerator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum InferenceError {
    #[error("local model runtime is not installed: {0}")]
    RuntimeMissing(String),
    #[error("local model runtime stopped unexpectedly: {0}")]
    WorkerCrashed(String),
    #[error("local model timed out after {0} ms")]
    Timeout(u64),
    #[error("cancelled")]
    Cancelled,
    #[error("not enough memory to run the local model: {0}")]
    OutOfMemory(String),
    #[error("local model could not load: {0}")]
    LoadFailed(String),
    #[error("invalid inference request: {0}")]
    BadRequest(String),
    #[error("local model runtime protocol error: {0}")]
    Protocol(String),
}

/// Runs one loaded model at a time. Implementations must be bounded
/// (timeouts, output caps) and honor `cancelled` promptly.
pub trait InferenceBackend: Send {
    fn info(&self) -> BackendInfo;

    /// Load `model` unless it is already the loaded one.
    fn ensure_loaded(
        &mut self,
        model: &VerifiedModel,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<LoadReport>, InferenceError>;

    fn generate(
        &mut self,
        req: &GenerateRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GenerateResponse, InferenceError>;

    fn memory(&mut self) -> Result<BackendMemory, InferenceError>;

    /// The model currently loaded, if any.
    fn loaded_model_id(&self) -> Option<&str> {
        None
    }

    /// OS process running inference, when separate (memory accounting).
    fn process_id(&self) -> Option<i32> {
        None
    }

    /// Free the model (the process may stay up).
    fn unload(&mut self) -> Result<(), InferenceError>;

    /// Stop everything this backend runs.
    fn shutdown(&mut self);
}

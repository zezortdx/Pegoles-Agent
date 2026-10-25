//! Pegoles local inference: everything needed to run a vision-language
//! model on the HOST, and nothing about what its answers mean.
//!
//! ```text
//! hardware  ── what this Mac is, how much memory is in use
//! catalog   ── pinned model entries (source, license, bytes, digests)
//! store     ── verified install / resume / remove under <data>/models
//! backend   ── InferenceBackend trait (replaceable)
//! worker    ── MLX backend (macOS): supervised persistent worker
//! llama     ── llama.cpp backend (Windows; also runs on macOS): same
//!              protocol and supervision, sandboxed per platform
//! ```
//!
//! Model output is plain text here. Turning it into actions (strict
//! parsing, typed `ComputerAction`s, policy) is the agent's job; this
//! crate never interprets it and never gives it a code path.

pub mod backend;
pub mod catalog;
pub mod download;
pub mod hardware;
pub mod llama;
#[cfg(windows)]
mod sandbox_windows;
pub mod store;
pub(crate) mod supervisor;
#[cfg(unix)]
pub mod worker;

pub use backend::{
    BackendInfo, BackendMemory, ChatMessage, GenerateRequest, GenerateResponse, ImageInput,
    InferenceBackend, InferenceError, LoadReport, Part, Role, Timings,
};
pub use catalog::{Catalog, ModelFamily, ModelFormat, ModelSpec};
pub use llama::{LlamaWorkerBackend, LlamaWorkerConfig};
pub use store::{
    InstallPhase, InstallProgress, InstallState, ModelStore, StoreError, VerifiedModel,
};
#[cfg(unix)]
pub use worker::{MlxWorkerBackend, MlxWorkerConfig};

/// `<data>/models`.
pub fn models_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("models")
}

//! Pegoles local inference: everything needed to run a vision-language
//! model on the HOST, and nothing about what its answers mean.
//!
//! ```text
//! hardware  ── what this Mac is, how much memory is in use
//! catalog   ── pinned model entries (source, license, bytes, digests)
//! store     ── verified install / resume / remove under <data>/models
//! backend   ── InferenceBackend trait (replaceable)
//! worker    ── MLX reference backend: supervised persistent worker
//! ```
//!
//! Model output is plain text here. Turning it into actions (strict
//! parsing, typed `ComputerAction`s, policy) is the agent's job; this
//! crate never interprets it and never gives it a code path.

pub mod backend;
pub mod catalog;
pub mod download;
pub mod hardware;
pub mod store;
pub mod worker;

pub use backend::{
    BackendInfo, BackendMemory, ChatMessage, GenerateRequest, GenerateResponse, ImageInput,
    InferenceBackend, InferenceError, LoadReport, Part, Role, Timings,
};
pub use catalog::{Catalog, ModelFamily, ModelSpec};
pub use store::{
    InstallPhase, InstallProgress, InstallState, ModelStore, StoreError, VerifiedModel,
};
pub use worker::{MlxWorkerBackend, MlxWorkerConfig};

/// `<data>/models`.
pub fn models_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("models")
}

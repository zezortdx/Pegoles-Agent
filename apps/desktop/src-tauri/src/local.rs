//! Pegoles Local in the desktop app: the model store (download, verify,
//! atomic install, remove) with progress events, and the persistent
//! inference worker shared by every run.
//!
//! The worker runs on the HOST (never in the guest), keeps the model
//! loaded between tasks, and only ever returns text: the local planner
//! parses it into typed actions that go through Pegoles Policy like any
//! other provider's. Which worker depends on the model's format: MLX
//! models run in the MLX worker (macOS), GGUF models in the llama.cpp
//! worker (Windows; it also runs on macOS).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pegoles_agent::local::{LocalConfig, LocalPlanner, SharedBackend};
use pegoles_inference::download::HttpsFetcher;
use pegoles_inference::{
    hardware, models_dir, Catalog, InferenceBackend, InferenceError, InstallPhase, InstallProgress,
    InstallState, LlamaWorkerBackend, LlamaWorkerConfig, ModelFormat, ModelSpec, ModelStore,
    StoreError, VerifiedModel,
};
#[cfg(unix)]
use pegoles_inference::{MlxWorkerBackend, MlxWorkerConfig};
use serde::Serialize;
use tauri::Emitter;

pub const INSTALL_EVENT: &str = "pegoles://model-install";
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// The worker (and its ~2 GB model) is stopped after this long unused.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);

fn data_dir() -> std::path::PathBuf {
    pegoles_computer::pegoles_data_dir()
}

pub fn store() -> ModelStore {
    ModelStore::new(models_dir(&data_dir()))
}

/// Why Pegoles Local cannot run on this computer at all, in plain words.
fn host_problem(hw: &hardware::HardwareProfile) -> Option<String> {
    if cfg!(target_os = "macos") {
        (!hw.apple_silicon).then(|| "Pegoles Local needs a Mac with Apple silicon.".into())
    } else if cfg!(windows) {
        (hw.arch != "x86_64")
            .then(|| "Pegoles Local on Windows needs a 64-bit Intel or AMD PC.".into())
    } else {
        Some("Pegoles Local is not available on this system yet.".into())
    }
}

#[cfg(not(unix))]
fn mac_only() -> InferenceError {
    InferenceError::RuntimeMissing("this model runs on a Mac only".into())
}

/// Whether the worker for `spec` is installed (cheap: nothing starts).
fn runtime_check(spec: &ModelSpec) -> Result<(), InferenceError> {
    match spec.format {
        #[cfg(unix)]
        ModelFormat::Mlx => MlxWorkerConfig::discover(&data_dir()).map(|_| ()),
        #[cfg(not(unix))]
        ModelFormat::Mlx => Err(mac_only()),
        ModelFormat::Gguf => LlamaWorkerConfig::discover(&data_dir()).map(|_| ()),
    }
}

/// A new (not yet started) worker for `spec`'s format.
fn new_backend(spec: &ModelSpec) -> Result<Box<dyn InferenceBackend>, InferenceError> {
    Ok(match spec.format {
        #[cfg(unix)]
        ModelFormat::Mlx => Box::new(MlxWorkerBackend::new(MlxWorkerConfig::discover(
            &data_dir(),
        )?)),
        #[cfg(not(unix))]
        ModelFormat::Mlx => return Err(mac_only()),
        ModelFormat::Gguf => Box::new(LlamaWorkerBackend::new(LlamaWorkerConfig::discover(
            &data_dir(),
        )?)),
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SetupPhase {
    Idle,
    Downloading,
    Verifying,
    Finalizing,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct InstallStatus {
    pub model: String,
    pub phase: SetupPhase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// Plain-language reason when `phase` is `failed`.
    pub error: Option<String>,
    /// `disk_space`, `network`, `corrupted`, `other`.
    pub error_kind: Option<&'static str>,
}

#[derive(Default)]
struct Inner {
    job: Option<(String, Arc<AtomicBool>)>,
    status: Option<InstallStatus>,
    backend: Option<SharedBackend>,
    /// The model format `backend` runs.
    backend_format: Option<ModelFormat>,
    /// Models whose bytes were fully verified in this app session.
    verified: HashMap<String, VerifiedModel>,
    runs: u32,
    last_used: Option<Instant>,
}

/// Held by a run while it may use the worker.
pub struct InUse(LocalModels);

impl Drop for InUse {
    fn drop(&mut self) {
        let mut inner = self.0.inner();
        inner.runs = inner.runs.saturating_sub(1);
        inner.last_used = Some(Instant::now());
    }
}

/// App-managed state (Tauri `manage`). Cheap to clone.
#[derive(Clone, Default)]
pub struct LocalModels(Arc<Mutex<Inner>>);

#[derive(Clone, Debug, Serialize)]
pub struct LocalModelPayload {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub parameters: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub license: String,
    pub source: String,
    /// `not_installed`, `partial`, `installed`, `invalid`.
    pub state: &'static str,
    pub partial_bytes: Option<u64>,
    pub invalid_reason: Option<String>,
    pub downloadable: bool,
    pub recommended_min_ram_gb: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalRuntimePayload {
    /// The inference runtime (MLX worker on macOS, llama.cpp worker on
    /// Windows) is set up on this computer.
    pub runtime_ready: bool,
    pub runtime_problem: Option<String>,
    /// The worker is running with this model loaded.
    pub loaded_model: Option<String>,
    pub worker_footprint_bytes: Option<u64>,
    pub default_model: String,
    pub models: Vec<LocalModelPayload>,
    pub install: Option<InstallStatus>,
    pub chip: Option<String>,
    pub memory_bytes: u64,
    pub apple_silicon: bool,
    /// This computer can run Pegoles Local at all (the UI's "unsupported").
    pub host_supported: bool,
}

fn model_payload(store: &ModelStore, spec: &ModelSpec) -> LocalModelPayload {
    let (state, partial_bytes, invalid_reason) = match store.state(spec) {
        InstallState::NotInstalled => ("not_installed", None, None),
        InstallState::Partial { bytes, .. } => ("partial", Some(bytes), None),
        InstallState::Installed => ("installed", None, None),
        InstallState::Invalid { reason } => ("invalid", None, Some(reason)),
    };
    let source = match &spec.source {
        pegoles_inference::catalog::ModelSource::Huggingface { repo, revision } => {
            format!("huggingface.co/{repo} @ {}", &revision[..12])
        }
        pegoles_inference::catalog::ModelSource::LocalConversion { upstream_repo, .. } => {
            format!("local conversion of {upstream_repo}")
        }
    };
    LocalModelPayload {
        id: spec.id.clone(),
        display_name: spec.display_name.clone(),
        family: format!("{:?}", spec.family),
        parameters: spec.parameters.clone(),
        quantization: spec.quantization.clone(),
        size_bytes: spec.total_bytes(),
        license: spec.license.clone(),
        source,
        state,
        partial_bytes,
        invalid_reason,
        downloadable: spec.downloadable(),
        recommended_min_ram_gb: spec.recommended_min_ram_gb,
    }
}

fn error_kind(e: &StoreError) -> &'static str {
    match e {
        StoreError::DiskSpace { .. } => "disk_space",
        StoreError::Network(_) => "network",
        StoreError::Corrupted { .. } | StoreError::Invalid { .. } => "corrupted",
        _ => "other",
    }
}

fn plain(e: &StoreError) -> String {
    match e {
        StoreError::DiskSpace {
            needed_bytes,
            free_bytes,
        } => format!(
            "Not enough disk space: Pegoles Local needs {:.1} GB free and {:.1} GB is available.",
            *needed_bytes as f64 / 1e9,
            *free_bytes as f64 / 1e9
        ),
        StoreError::Network(_) => {
            "The download was interrupted. Check your connection and retry; it resumes where it stopped.".into()
        }
        StoreError::Corrupted { .. } => {
            "The downloaded model didn’t match its checksum and was discarded. Retry to download it again.".into()
        }
        other => other.to_string(),
    }
}

/// Bytes kept on disk for a resumable download.
fn partial_bytes(store: &ModelStore, spec: &ModelSpec) -> u64 {
    match store.state(spec) {
        InstallState::Partial { bytes, .. } => bytes,
        _ => 0,
    }
}

/// The runtime problem in plain words (no error-type prefix).
fn runtime_problem_text(e: &pegoles_inference::InferenceError) -> String {
    match e {
        pegoles_inference::InferenceError::RuntimeMissing(m) => {
            let mut m = m.clone();
            if let Some(first) = m.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            if !m.ends_with('.') {
                m.push('.');
            }
            m
        }
        other => other.to_string(),
    }
}

impl LocalModels {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn payload(&self) -> LocalRuntimePayload {
        let catalog = Catalog::builtin();
        let store = store();
        let runtime = runtime_check(catalog.default_spec());
        let hw = hardware::detect();
        let host_problem = host_problem(&hw);
        let host_supported = host_problem.is_none();
        let (install, backend) = {
            let inner = self.inner();
            (inner.status.clone(), inner.backend.clone())
        };
        let (loaded_model, footprint) = backend
            .and_then(|b| {
                let guard = b.try_lock().ok()?;
                Some((
                    guard.loaded_model_id().map(str::to_string),
                    guard.process_id(),
                ))
            })
            .map_or((None, None), |(m, pid)| {
                (
                    m,
                    pid.and_then(hardware::process_memory)
                        .map(|m| m.phys_footprint_bytes),
                )
            });
        LocalRuntimePayload {
            runtime_ready: runtime.is_ok() && host_problem.is_none(),
            runtime_problem: host_problem
                .or_else(|| runtime.err().map(|e| runtime_problem_text(&e))),
            loaded_model,
            worker_footprint_bytes: footprint,
            default_model: catalog.default_model.clone(),
            // Only offered models (benchmark winners) are listed, plus any
            // other catalog model already installed on this Mac.
            models: catalog
                .models
                .iter()
                .map(|m| model_payload(&store, m))
                .filter(|m| (catalog.is_offered(&m.id) && m.downloadable) || m.state == "installed")
                .collect(),
            install,
            chip: hw.chip,
            memory_bytes: hw.total_memory_bytes,
            apple_silicon: hw.apple_silicon,
            host_supported,
        }
    }

    /// Whether `model` can run now (installed + runtime present). Cheap.
    pub fn ready(&self, model: &str) -> bool {
        let Some(spec) = Catalog::builtin().get(model).cloned() else {
            return false;
        };
        host_problem(&hardware::detect()).is_none()
            && runtime_check(&spec).is_ok()
            && store().state(&spec) == InstallState::Installed
    }

    pub fn start_install(
        &self,
        app: tauri::AppHandle,
        shared: crate::commands::SharedState,
        model: String,
    ) -> Result<(), String> {
        let catalog = Catalog::builtin();
        if !catalog.is_offered(&model) {
            return Err(format!("{model:?} is not offered for installation"));
        }
        let spec = catalog
            .get(&model)
            .cloned()
            .ok_or_else(|| format!("unknown model {model:?}"))?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut inner = self.inner();
            if let Some((running, _)) = &inner.job {
                return Err(format!("{running} is already being set up"));
            }
            inner.job = Some((model.clone(), cancel.clone()));
            inner.status = Some(InstallStatus {
                model: model.clone(),
                phase: SetupPhase::Downloading,
                done_bytes: 0,
                total_bytes: spec.total_bytes(),
                error: None,
                error_kind: None,
            });
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("pegoles-model-install".into())
            .spawn(move || {
                me.install_thread(app, spec, cancel);
                // The status payload's "can run a task" flag follows.
                crate::commands::refresh_readiness(&shared, &me);
            })
            .map(|_| ())
            .map_err(|e| {
                self.inner().job = None;
                format!("could not start the download: {e}")
            })
    }

    fn publish(&self, app: &tauri::AppHandle, status: InstallStatus) {
        self.inner().status = Some(status.clone());
        let _ = app.emit(INSTALL_EVENT, &status);
    }

    fn install_thread(&self, app: tauri::AppHandle, spec: ModelSpec, cancel: Arc<AtomicBool>) {
        let store = store();
        let mut last = Instant::now() - PROGRESS_EVERY;
        let model = spec.id.clone();
        let result = store.install(
            &spec,
            &HttpsFetcher::default(),
            &mut |p: &InstallProgress| {
                let phase_changed = p.phase != InstallPhase::Downloading;
                if phase_changed || last.elapsed() >= PROGRESS_EVERY {
                    last = Instant::now();
                    self.publish(
                        &app,
                        InstallStatus {
                            model: model.clone(),
                            phase: match p.phase {
                                InstallPhase::Downloading => SetupPhase::Downloading,
                                InstallPhase::Verifying => SetupPhase::Verifying,
                                InstallPhase::Finalizing => SetupPhase::Finalizing,
                            },
                            done_bytes: p.done_bytes,
                            total_bytes: p.total_bytes,
                            error: None,
                            error_kind: None,
                        },
                    );
                }
            },
            &|| cancel.load(Ordering::SeqCst),
        );
        let status = match result {
            Ok(verified) => {
                self.inner().verified.insert(model.clone(), verified);
                InstallStatus {
                    model: model.clone(),
                    phase: SetupPhase::Ready,
                    done_bytes: spec.total_bytes(),
                    total_bytes: spec.total_bytes(),
                    error: None,
                    error_kind: None,
                }
            }
            Err(StoreError::Cancelled) => InstallStatus {
                model: model.clone(),
                phase: SetupPhase::Cancelled,
                done_bytes: partial_bytes(&store, &spec),
                total_bytes: spec.total_bytes(),
                error: None,
                error_kind: None,
            },
            Err(e) => InstallStatus {
                model: model.clone(),
                phase: SetupPhase::Failed,
                done_bytes: partial_bytes(&store, &spec),
                total_bytes: spec.total_bytes(),
                error: Some(plain(&e)),
                error_kind: Some(error_kind(&e)),
            },
        };
        self.inner().job = None;
        self.publish(&app, status);
    }

    pub fn cancel_install(&self) -> bool {
        match &self.inner().job {
            Some((_, cancel)) => {
                cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Remove a model (and any partial download). Stops the worker if it
    /// has that model loaded.
    pub fn remove(&self, model: &str) -> Result<(), String> {
        if Catalog::builtin().get(model).is_none() {
            return Err(format!("unknown local model {model:?}"));
        }
        if self.inner().job.as_ref().is_some_and(|(m, _)| m == model) {
            return Err("cancel the download first".into());
        }
        let backend = {
            let mut inner = self.inner();
            inner.verified.remove(model);
            inner.status = None;
            inner.backend.clone()
        };
        if let Some(b) = backend {
            let mut guard = b.try_lock().map_err(|_| "the model is in use by a task")?;
            if guard.loaded_model_id() == Some(model) {
                guard.shutdown();
            }
        }
        store().remove(model).map_err(|e| e.to_string())
    }

    /// A planner for one run: the model's bytes verified (fully, once per
    /// app session), the shared worker created on first use.
    pub fn planner(&self, model: &str) -> Result<LocalPlanner, String> {
        let spec = Catalog::builtin()
            .get(model)
            .cloned()
            .ok_or_else(|| format!("unknown local model {model:?}"))?;
        let cached = self.inner().verified.get(model).cloned();
        let verified = match cached {
            Some(v) if store().state(&spec) == InstallState::Installed => v,
            _ => {
                let v = store().verify(&spec).map_err(|e| match e {
                    StoreError::NotInstalled(_) => {
                        "Set up Pegoles Local in Settings first.".to_string()
                    }
                    other => format!(
                        "The local model failed its integrity check ({other}). Remove it and set it up again in Settings."
                    ),
                })?;
                self.inner().verified.insert(model.to_string(), v.clone());
                v
            }
        };
        let reuse = {
            let inner = self.inner();
            inner
                .backend
                .clone()
                .filter(|_| inner.backend_format == Some(spec.format))
        };
        let backend = match reuse {
            Some(b) => b,
            None => {
                // A model of another format: stop the other worker first.
                self.shutdown();
                let b: SharedBackend = Arc::new(Mutex::new(
                    new_backend(&spec).map_err(|e| runtime_problem_text(&e))?,
                ));
                let mut inner = self.inner();
                inner.backend = Some(b.clone());
                inner.backend_format = Some(spec.format);
                b
            }
        };
        Ok(LocalPlanner::new(LocalConfig::for_model(verified), backend))
    }

    pub fn in_use(&self) -> InUse {
        self.inner().runs += 1;
        InUse(self.clone())
    }

    /// Free the model's memory when nothing has used it for a while.
    pub fn idle_check(&self) {
        let idle = {
            let inner = self.inner();
            inner.runs == 0
                && inner.backend.is_some()
                && inner.last_used.is_some_and(|t| t.elapsed() > IDLE_UNLOAD)
        };
        if idle {
            self.shutdown();
        }
    }

    /// Verify the chosen model's bytes in the background at startup, so
    /// the first task does not wait for hashing.
    pub fn prewarm_verify(&self, model: &str) {
        let Some(spec) = Catalog::builtin().get(model).cloned() else {
            return;
        };
        if store().state(&spec) != InstallState::Installed {
            return;
        }
        if let Ok(v) = store().verify(&spec) {
            self.inner().verified.insert(model.to_string(), v);
        }
    }

    /// Stop the worker (app exit, or to free memory).
    pub fn shutdown(&self) {
        let backend = {
            let mut inner = self.inner();
            inner.backend_format = None;
            inner.backend.take()
        };
        if let Some(b) = backend {
            if let Ok(mut g) = b.try_lock() {
                g.shutdown();
            }
        }
    }
}

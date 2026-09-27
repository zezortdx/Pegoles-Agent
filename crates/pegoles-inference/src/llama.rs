//! llama.cpp backend: a persistent, supervised `pegoles-llm-worker`.
//!
//! ```text
//! LlamaWorkerBackend ──stdin JSONL (v1)──▶ pegoles-llm-worker ─▶ llama.cpp ─▶ Vulkan / Metal / CPU
//!                    ◀─stdout JSONL──────
//! ```
//!
//! Same protocol, bounds and supervision rules as the MLX worker
//! (`worker.rs`): the model stays loaded, every request is bounded,
//! cancellation is cooperative then forceful, a crashed worker is
//! respawned and the model re-verified before it is loaded again. The
//! worker runs with a cleared environment and no network:
//!
//! - Windows: an AppContainer with no capabilities (no network, no user
//!   files but the model folder it is granted), inside a Job Object that
//!   kills it with Pegoles, allows one process only and caps its memory
//!   (`sandbox_windows.rs`);
//! - macOS: `sandbox-exec` with a profile like the MLX worker's.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

use crate::backend::{
    BackendInfo, BackendMemory, GenerateRequest, GenerateResponse, InferenceBackend,
    InferenceError, LoadReport, Timings,
};
use crate::store::{ModelStore, VerifiedModel};
use crate::supervisor::{
    drain_stderr, read_replies, wire_messages, write_requests, Line, PROTOCOL_VERSION,
};

/// The worker executable's file name.
pub const WORKER_EXE: &str = if cfg!(windows) {
    "pegoles-llm-worker.exe"
} else {
    "pegoles-llm-worker"
};
const POLL: Duration = Duration::from_millis(25);
const EXIT_GRACE: Duration = Duration::from_secs(1);
const STDERR_SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
pub struct LlamaWorkerConfig {
    pub worker: PathBuf,
    /// The model store root (the only model location the worker can read).
    pub models_root: PathBuf,
    /// Run sandboxed (AppContainer + job on Windows, sandbox-exec on
    /// macOS). Mandatory in production; supervisor tests turn it off.
    pub sandbox: bool,
    /// Force the CPU even when a GPU backend is present.
    pub cpu_only: bool,
    pub hello_timeout: Duration,
    pub load_timeout: Duration,
    pub cancel_grace: Duration,
    /// Committed-memory cap for the worker (Windows job object).
    pub memory_limit_bytes: u64,
}

impl LlamaWorkerConfig {
    pub fn new(worker: PathBuf, models_root: PathBuf) -> Self {
        Self {
            worker,
            models_root,
            sandbox: true,
            cpu_only: false,
            hello_timeout: Duration::from_secs(60),
            load_timeout: Duration::from_secs(300),
            cancel_grace: Duration::from_secs(3),
            memory_limit_bytes: 12 * 1024 * 1024 * 1024,
        }
    }

    /// The worker shipped next to the app (Windows: the install folder;
    /// macOS: `Contents/MacOS`). Debug builds also accept the repository's
    /// build output. Never a copy in a user-writable data folder.
    pub fn discover(data_dir: &Path) -> Result<Self, InferenceError> {
        let mut candidates = Vec::new();
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
        {
            candidates.push(dir.join(WORKER_EXE));
        }
        if cfg!(debug_assertions) {
            candidates.push(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../workers/llama/target/release")
                    .join(WORKER_EXE),
            );
        }
        let worker = candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
            InferenceError::RuntimeMissing(
                "the Pegoles Local runtime is missing from this installation; reinstall Pegoles".into(),
            )
        })?;
        #[cfg(target_os = "macos")]
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(InferenceError::RuntimeMissing(
                "macOS sandboxing (sandbox-exec) is unavailable, so the model cannot run safely"
                    .into(),
            ));
        }
        Ok(Self::new(
            worker.canonicalize().unwrap_or(worker),
            crate::models_dir(data_dir),
        ))
    }
}

/// What the worker said about itself.
#[derive(Clone, Debug, Default)]
pub struct LlamaHello {
    pub worker_version: String,
    /// llama.cpp backend in use: `Vulkan`, `CUDA`, `Metal`, `CPU`.
    pub backend: String,
    pub device: String,
    pub gpu: bool,
}

/// A running worker process, whatever the platform.
pub(crate) trait ChildProc: Send {
    fn pid(&self) -> Option<i32>;
    /// Exited, without reaping it.
    fn exited(&mut self) -> bool;
    /// Kill it (and anything it started) and reap it.
    fn terminate(&mut self) -> Option<i32>;
}

/// The pipes of a freshly spawned worker.
pub(crate) struct Spawned {
    pub child: Box<dyn ChildProc>,
    pub stdin: Box<dyn std::io::Write + Send>,
    pub stdout: Box<dyn std::io::Read + Send>,
    pub stderr: Box<dyn std::io::Read + Send>,
}

struct Session {
    child: Box<dyn ChildProc>,
    stdin: Sender<Vec<u8>>,
    rx: Receiver<Line>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_done: Receiver<()>,
    reaped: bool,
}

impl Session {
    fn start(spawned: Spawned) -> Result<Self, InferenceError> {
        let (tx, rx) = mpsc::channel();
        let (lines_tx, lines_rx) = mpsc::channel();
        let (done_tx, stderr_done) = mpsc::channel::<()>();
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let sink = stderr.clone();
        let Spawned {
            child,
            stdin,
            stdout,
            stderr: stderr_pipe,
        } = spawned;
        let session = Session {
            child,
            stdin: lines_tx,
            rx,
            stderr,
            stderr_done,
            reaped: false,
        };
        let failed = |e: std::io::Error| InferenceError::WorkerCrashed(e.to_string());
        std::thread::Builder::new()
            .name("pegoles-llm-stdin".into())
            .spawn(move || write_requests(stdin, lines_rx))
            .map_err(failed)?;
        std::thread::Builder::new()
            .name("pegoles-llm-stdout".into())
            .spawn(move || read_replies(stdout, tx))
            .map_err(failed)?;
        std::thread::Builder::new()
            .name("pegoles-llm-stderr".into())
            .spawn(move || {
                drain_stderr(stderr_pipe, &sink);
                drop(done_tx);
            })
            .map_err(failed)?;
        Ok(session)
    }

    fn terminate(&mut self) -> Option<i32> {
        self.reaped = true;
        self.child.terminate()
    }

    /// Let it exit on its own within `grace`, then kill it.
    fn reap(&mut self, grace: Duration) -> Option<i32> {
        let deadline = Instant::now() + grace;
        while !self.child.exited() && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        self.terminate()
    }

    fn stderr_tail(&self, n: usize) -> String {
        let lines = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let skip = lines.len().saturating_sub(n);
        lines
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.reaped {
            self.child.terminate();
        }
    }
}

pub struct LlamaWorkerBackend {
    cfg: LlamaWorkerConfig,
    session: Option<Session>,
    next_id: u64,
    loaded: Option<VerifiedModel>,
    hello: Option<LlamaHello>,
    pub restarts: u32,
}

impl LlamaWorkerBackend {
    pub fn new(cfg: LlamaWorkerConfig) -> Self {
        Self {
            cfg,
            session: None,
            next_id: 1,
            loaded: None,
            hello: None,
            restarts: 0,
        }
    }

    pub fn hello_info(&self) -> Option<&LlamaHello> {
        self.hello.as_ref()
    }

    pub fn pid(&self) -> Option<i32> {
        self.session.as_ref().and_then(|s| s.child.pid())
    }

    fn spawn(&mut self) -> Result<(), InferenceError> {
        if !self.cfg.worker.is_file() {
            return Err(InferenceError::RuntimeMissing(format!(
                "worker not found at {}",
                self.cfg.worker.display()
            )));
        }
        let spawned = spawn_worker(&self.cfg)?;
        self.session = Some(Session::start(spawned)?);
        self.next_id = 1;
        self.loaded = None;
        let reply = self.request(json!({"op": "hello"}), self.cfg.hello_timeout, &|| false)?;
        if reply.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL_VERSION)
            || reply.get("worker").and_then(Value::as_str) != Some("pegoles-llama")
        {
            self.kill();
            return Err(InferenceError::Protocol(
                "the worker speaks a different protocol".into(),
            ));
        }
        let s = |k: &str| {
            reply
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .chars()
                .take(80)
                .collect::<String>()
        };
        self.hello = Some(LlamaHello {
            worker_version: s("worker_version"),
            backend: s("backend"),
            device: s("device"),
            gpu: reply.get("gpu").and_then(Value::as_bool) == Some(true),
        });
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(mut s) = self.session.take() {
            s.terminate();
        }
        self.loaded = None;
    }

    fn ensure_running(&mut self) -> Result<(), InferenceError> {
        if let Some(s) = self.session.as_mut() {
            if !s.child.exited() {
                return Ok(());
            }
            self.kill();
            self.restarts += 1;
        }
        self.spawn()
    }

    /// One request/response exchange (same rules as the MLX supervisor):
    /// the worker is killed on a timeout, on a cancel it does not honor
    /// within the grace period, and on any protocol violation.
    fn request(
        &mut self,
        mut body: Value,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, InferenceError> {
        let id = self.next_id;
        self.next_id += 1;
        body["v"] = json!(PROTOCOL_VERSION);
        body["id"] = json!(id);
        let mut line =
            serde_json::to_vec(&body).map_err(|e| InferenceError::BadRequest(e.to_string()))?;
        line.push(b'\n');
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| InferenceError::WorkerCrashed("worker is not running".into()))?;
        if session.stdin.send(line).is_err() {
            let tail = session.stderr_tail(4);
            self.kill();
            return Err(InferenceError::WorkerCrashed(format!(
                "the worker closed its input; {tail}"
            )));
        }
        let started = Instant::now();
        let mut cancel_deadline: Option<Instant> = None;
        loop {
            let session = self.session.as_mut().expect("present during request");
            match session.rx.recv_timeout(POLL) {
                Ok(Line::Reply(v)) => {
                    if v.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if v.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(v);
                    }
                    let kind = v
                        .pointer("/error/kind")
                        .and_then(Value::as_str)
                        .unwrap_or("internal");
                    let msg: String = v
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .take(300)
                        .collect();
                    return Err(match kind {
                        "bad_request" => InferenceError::BadRequest(msg),
                        "out_of_memory" => InferenceError::OutOfMemory(msg),
                        "runtime_missing" => InferenceError::RuntimeMissing(msg),
                        _ => InferenceError::Protocol(msg),
                    });
                }
                Ok(Line::Oversized) | Ok(Line::Invalid) => {
                    self.kill();
                    return Err(InferenceError::Protocol(
                        "worker sent an invalid or oversized reply".into(),
                    ));
                }
                Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    let mut session = self.session.take().expect("present during request");
                    let status = session.reap(EXIT_GRACE);
                    let _ = session.stderr_done.recv_timeout(STDERR_SETTLE);
                    let tail = session.stderr_tail(6);
                    drop(session);
                    self.loaded = None;
                    let msg = format!(
                        "exit {status:?}{}",
                        if tail.is_empty() {
                            String::new()
                        } else {
                            format!(": {tail}")
                        }
                    );
                    return Err(if tail.to_lowercase().contains("memory") {
                        InferenceError::OutOfMemory(msg)
                    } else {
                        InferenceError::WorkerCrashed(msg)
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(deadline) = cancel_deadline {
                if Instant::now() > deadline {
                    self.kill();
                    return Err(InferenceError::Cancelled);
                }
            } else if cancelled() {
                let cancel = json!({"v": PROTOCOL_VERSION, "id": 0, "op": "cancel", "target": id});
                let session = self.session.as_mut().expect("present");
                let _ = session.stdin.send(format!("{cancel}\n").into_bytes());
                cancel_deadline = Some(Instant::now() + self.cfg.cancel_grace);
            }
            if started.elapsed() > timeout {
                self.kill();
                return Err(InferenceError::Timeout(timeout.as_millis() as u64));
            }
        }
    }

    fn memory_now(&self) -> BackendMemory {
        let proc_mem = self.pid().and_then(crate::hardware::process_memory);
        BackendMemory {
            active_bytes: 0,
            peak_bytes: 0,
            cache_bytes: 0,
            process_footprint_bytes: proc_mem.map(|m| m.phys_footprint_bytes),
            process_peak_footprint_bytes: proc_mem.map(|m| m.lifetime_max_phys_footprint_bytes),
        }
    }
}

impl InferenceBackend for LlamaWorkerBackend {
    fn info(&self) -> BackendInfo {
        let h = self.hello.clone().unwrap_or_default();
        BackendInfo {
            backend: "llama.cpp".into(),
            runtime_version: format!("pegoles-llm-worker {}", h.worker_version),
            accelerator: if h.backend.is_empty() {
                "unknown".into()
            } else {
                h.backend.to_lowercase()
            },
        }
    }

    fn ensure_loaded(
        &mut self,
        model: &VerifiedModel,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<LoadReport>, InferenceError> {
        let gguf =
            model.spec.gguf.clone().ok_or_else(|| {
                InferenceError::LoadFailed("this model is not a GGUF model".into())
            })?;
        self.ensure_running()?;
        if self.loaded.as_ref() == Some(model) {
            return Ok(None);
        }
        // Re-hash the store copy right before the worker reads it (once per
        // load, never per step), as for MLX.
        let fresh = ModelStore::new(self.cfg.models_root.clone())
            .verify(&model.spec)
            .map_err(|e| InferenceError::LoadFailed(e.to_string()))?;
        if fresh.dir != model.dir {
            return Err(InferenceError::LoadFailed(
                "the model is not in the model store".into(),
            ));
        }
        let dir = model
            .dir
            .to_str()
            .ok_or_else(|| InferenceError::BadRequest("model path is not UTF-8".into()))?;
        let load = json!({"op": "load", "model_dir": dir, "model_file": gguf.model, "projector_file": gguf.projector});
        let reply = match self.request(load, self.cfg.load_timeout, cancelled) {
            Ok(r) => r,
            Err(InferenceError::Protocol(m)) | Err(InferenceError::BadRequest(m)) => {
                return Err(InferenceError::LoadFailed(m))
            }
            Err(e) => return Err(e),
        };
        self.loaded = Some(model.clone());
        Ok(Some(LoadReport {
            model_id: model.spec.id.clone(),
            load_ms: reply.get("load_ms").and_then(Value::as_f64).unwrap_or(0.0),
            verify_ms: fresh.verify_ms,
            memory: self.memory_now(),
        }))
    }

    fn generate(
        &mut self,
        req: &GenerateRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GenerateResponse, InferenceError> {
        if self.loaded.is_none() {
            return Err(InferenceError::BadRequest("no model is loaded".into()));
        }
        self.ensure_running()?;
        if self.loaded.is_none() {
            return Err(InferenceError::WorkerCrashed(
                "worker restarted; model must be reloaded".into(),
            ));
        }
        let engine = base64::engine::general_purpose::STANDARD;
        let images: Vec<Value> = req
            .images
            .iter()
            .map(|i| Value::String(engine.encode(i.png.as_slice())))
            .collect();
        let prep: Vec<Value> = req
            .images
            .iter()
            .map(|i| {
                let mut p = json!({});
                if let Some(c) = i.crop {
                    p["crop"] = json!(c);
                }
                if let Some((w, h)) = i.resize {
                    p["resize"] = json!([w, h]);
                }
                p
            })
            .collect();
        let started = Instant::now();
        let reply = self.request(
            json!({
                "op": "generate",
                "messages": wire_messages(&req.messages),
                "images": images,
                "image_prep": prep,
                "max_tokens": req.max_tokens,
                "temperature": req.temperature,
            }),
            req.timeout,
            cancelled,
        )?;
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let finish: String = reply
            .get("finish")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(20)
            .collect();
        if finish == "cancelled" {
            return Err(InferenceError::Cancelled);
        }
        let f = |p: &str| reply.pointer(p).and_then(Value::as_f64).unwrap_or(0.0);
        let u = |k: &str| reply.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        let image_sizes = reply
            .get("image_sizes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| Some((s.get(0)?.as_u64()? as u32, s.get(1)?.as_u64()? as u32)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(GenerateResponse {
            text: reply
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            finish,
            prompt_tokens: u("prompt_tokens"),
            generation_tokens: u("generation_tokens"),
            prompt_tps: f("/prompt_tps"),
            generation_tps: f("/generation_tps"),
            image_sizes,
            timings: Timings {
                image_ms: f("/timings_ms/image"),
                first_token_ms: f("/timings_ms/first_token"),
                generate_ms: f("/timings_ms/generate"),
                total_ms: f("/timings_ms/total"),
                wall_ms,
            },
            memory: self.memory_now(),
        })
    }

    fn loaded_model_id(&self) -> Option<&str> {
        self.loaded.as_ref().map(|m| m.spec.id.as_str())
    }

    fn process_id(&self) -> Option<i32> {
        self.pid()
    }

    fn memory(&mut self) -> Result<BackendMemory, InferenceError> {
        Ok(self.memory_now())
    }

    fn unload(&mut self) -> Result<(), InferenceError> {
        if self.session.is_some() {
            self.request(json!({"op": "unload"}), Duration::from_secs(30), &|| false)?;
        }
        self.loaded = None;
        Ok(())
    }

    fn shutdown(&mut self) {
        if self.session.is_some() {
            let _ = self.request(json!({"op": "shutdown"}), Duration::from_secs(3), &|| false);
            if let Some(mut s) = self.session.take() {
                s.reap(Duration::from_secs(2));
            }
        }
        self.loaded = None;
    }
}

impl Drop for LlamaWorkerBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Spawning ───────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Seatbelt profile for the llama.cpp worker on macOS (later rules win):
/// no network, no fork, no exec but itself, no Apple Events, no Mach
/// services but the Metal compiler, IOKit only for the GPU, no user files
/// but the model store and its own folder, writes only to a private temp
/// directory. Mirrors the MLX worker's profile.
#[cfg(target_os = "macos")]
const SANDBOX_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-fork)
(deny process-exec*)
(allow process-exec (literal (param "WORKER")))
(deny appleevent-send)
(deny signal)
(allow signal (target self))
(deny process-info*)
(allow process-info* (target self))
(deny iokit-open)
(allow iokit-open (iokit-user-client-class "AGXDeviceUserClient" "IOSurfaceRootUserClient"))
(deny mach-lookup)
(allow mach-lookup (global-name "com.apple.MTLCompilerService"))
(deny file-read-data (subpath (param "HOME")))
(allow file-read-data (literal (param "HOME")) (subpath (param "MODELS")) (subpath (param "WORKER_DIR")))
(deny file-write*)
(allow file-write* (subpath (param "TMP")) (literal "/dev/null") (literal "/dev/dtracehelper"))
"#;

#[cfg(unix)]
struct UnixChild {
    child: std::process::Child,
    pid: i32,
    reaped: bool,
}

#[cfg(unix)]
impl ChildProc for UnixChild {
    fn pid(&self) -> Option<i32> {
        Some(self.pid)
    }

    fn exited(&mut self) -> bool {
        // SAFETY: an all-zero siginfo_t is valid; WNOWAIT leaves it unreaped.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        #[cfg(target_vendor = "apple")]
        let pid = info.si_pid;
        #[cfg(not(target_vendor = "apple"))]
        // SAFETY: filled by waitid (or zeroed).
        let pid = unsafe { info.si_pid() };
        r != 0 || pid != 0
    }

    fn terminate(&mut self) -> Option<i32> {
        if !self.reaped {
            // SAFETY: signalling our worker's own process group.
            unsafe { libc::killpg(self.pid, libc::SIGKILL) };
            let _ = self.child.kill();
            self.reaped = true;
        }
        self.child.wait().ok().and_then(|s| s.code())
    }
}

#[cfg(unix)]
fn spawn_worker(cfg: &LlamaWorkerConfig) -> Result<Spawned, InferenceError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd;
    #[cfg(target_os = "macos")]
    {
        if cfg.sandbox {
            let tmp = crate::worker::fresh_private_tmp("pegoles-llama")?;
            let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            let param = |p: &Path| -> Result<String, InferenceError> {
                p.to_str()
                    .filter(|s| p.is_absolute() && !s.contains('"'))
                    .map(str::to_string)
                    .ok_or_else(|| {
                        InferenceError::RuntimeMissing(format!("unusable path {}", p.display()))
                    })
            };
            let home = std::env::var("HOME")
                .map_err(|_| InferenceError::RuntimeMissing("HOME is not set".into()))?;
            let worker_dir = cfg
                .worker
                .parent()
                .ok_or_else(|| InferenceError::RuntimeMissing("bad worker path".into()))?;
            let tmp_param = match (tmp.parent(), tmp.file_name()) {
                (Some(parent), Some(name)) => param(&canonical(parent).join(name))?,
                _ => return Err(InferenceError::RuntimeMissing("bad worker temp dir".into())),
            };
            cmd = Command::new(SANDBOX_EXEC);
            cmd.args([
                "-D".to_string(),
                format!("HOME={}", param(&canonical(Path::new(&home)))?),
                "-D".into(),
                format!("TMP={tmp_param}"),
                "-D".into(),
                format!("WORKER={}", param(&canonical(&cfg.worker))?),
                "-D".into(),
                format!("WORKER_DIR={}", param(&canonical(worker_dir))?),
                "-D".into(),
                format!("MODELS={}", param(&canonical(&cfg.models_root))?),
                "-p".into(),
                SANDBOX_PROFILE.into(),
            ])
            .arg(&cfg.worker)
            .env_clear()
            .env("TMPDIR", tmp);
        } else {
            cmd = Command::new(&cfg.worker);
            cmd.env_clear();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if cfg.sandbox {
            return Err(InferenceError::RuntimeMissing(
                "no sandbox for the model worker on this OS".into(),
            ));
        }
        cmd = Command::new(&cfg.worker);
        cmd.env_clear();
    }
    cmd.env("PATH", "/usr/bin:/bin")
        .current_dir("/")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg.cpu_only {
        cmd.env("PEGOLES_LLM_CPU", "1");
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| InferenceError::RuntimeMissing(format!("cannot start worker: {e}")))?;
    let pid = child.id() as i32;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    Ok(Spawned {
        child: Box::new(UnixChild {
            child,
            pid,
            reaped: false,
        }),
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stderr: Box::new(stderr),
    })
}

#[cfg(windows)]
fn spawn_worker(cfg: &LlamaWorkerConfig) -> Result<Spawned, InferenceError> {
    crate::sandbox_windows::spawn(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_worker_is_a_clear_error() {
        let mut b = LlamaWorkerBackend::new(LlamaWorkerConfig::new(
            PathBuf::from("/nonexistent/pegoles-llm-worker"),
            PathBuf::from("/tmp"),
        ));
        assert!(matches!(
            b.ensure_running(),
            Err(InferenceError::RuntimeMissing(_))
        ));
    }

    /// The real worker, confined exactly as in production (AppContainer +
    /// job on Windows, sandbox-exec on macOS), answers `hello` and exits
    /// on `shutdown`. Opt-in: set PEGOLES_LLM_WORKER to the built worker
    /// (CI does on Windows).
    #[test]
    fn the_real_worker_starts_confined() {
        let Some(worker) = std::env::var_os("PEGOLES_LLM_WORKER").map(PathBuf::from) else {
            eprintln!("PEGOLES_LLM_WORKER not set; skipped");
            return;
        };
        let models = tempfile::tempdir().unwrap();
        let mut cfg =
            LlamaWorkerConfig::new(worker.canonicalize().unwrap(), models.path().to_path_buf());
        cfg.cpu_only = true;
        let mut b = LlamaWorkerBackend::new(cfg);
        b.ensure_running().expect("confined worker starts");
        let hello = b.hello_info().unwrap().clone();
        assert!(!hello.worker_version.is_empty() && hello.worker_version != "unknown");
        assert!(
            !hello.gpu,
            "PEGOLES_LLM_CPU=1 must keep it on the CPU: {hello:?}"
        );
        assert!(b.pid().is_some());
        b.shutdown();
        assert!(b.pid().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_stand_in_worker_is_supervised_like_the_real_one() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("worker");
        std::fs::write(
            &script,
            "#!/bin/sh\nread line\necho '{\"v\":1,\"id\":1,\"ok\":true,\"protocol\":1,\"worker\":\"pegoles-llama\",\"worker_version\":\"t\",\"backend\":\"CPU\",\"device\":\"\",\"gpu\":false}'\nread line\necho boom >&2\nexit 9\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = LlamaWorkerConfig::new(script, tmp.path().to_path_buf());
        cfg.sandbox = false;
        let mut b = LlamaWorkerBackend::new(cfg);
        b.ensure_running().unwrap();
        assert_eq!(b.hello_info().unwrap().backend, "CPU");
        let err = b.memory().and_then(|_| b.unload()).unwrap_err();
        assert!(
            matches!(err, InferenceError::WorkerCrashed(ref m) if m.contains("boom")),
            "{err:?}"
        );
        // Respawned on the next use.
        assert!(b.ensure_running().is_ok());
        assert_eq!(b.restarts, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_worker_speaking_another_protocol_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("worker");
        std::fs::write(&script, "#!/bin/sh\nread line\necho '{\"v\":1,\"id\":1,\"ok\":true,\"protocol\":1,\"worker\":\"pegoles-mlx\"}'\nsleep 5\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = LlamaWorkerConfig::new(script, tmp.path().to_path_buf());
        cfg.sandbox = false;
        let mut b = LlamaWorkerBackend::new(cfg);
        assert!(matches!(
            b.ensure_running(),
            Err(InferenceError::Protocol(_))
        ));
        assert!(b.pid().is_none());
    }
}

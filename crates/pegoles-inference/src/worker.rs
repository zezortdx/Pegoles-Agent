//! MLX backend: a persistent, supervised Python worker process.
//!
//! ```text
//! MlxWorkerBackend ──stdin JSONL (v1)──▶ pegoles_mlx_worker.py ─▶ mlx-vlm ─▶ Metal
//!                  ◀─stdout JSONL──────
//! ```
//!
//! The model stays loaded across requests. Every request is bounded
//! (request size, reply size, timeout); cancellation is cooperative first
//! (the worker stops between tokens) and forceful after a grace period
//! (kill). A crashed or killed worker is respawned on the next request
//! and the model reloaded. The worker runs with a cleared environment
//! (no API keys, offline flags), under `sandbox-exec` with the network
//! denied and user files read-only when available.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

use crate::backend::{
    BackendInfo, BackendMemory, ChatMessage, GenerateRequest, GenerateResponse, InferenceBackend,
    InferenceError, LoadReport, Part, Role, Timings,
};
use crate::hardware;
use crate::store::VerifiedModel;

pub const PROTOCOL_VERSION: u64 = 1;
/// Largest reply line accepted from the worker (text is capped at 32K
/// chars worker-side; this bounds a misbehaving or compromised worker).
const MAX_REPLY_BYTES: usize = 1024 * 1024;
const MAX_STDERR_LINES: usize = 40;
const MAX_STDERR_LINE_CHARS: usize = 400;
const POLL: Duration = Duration::from_millis(25);

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// MLX buffer-cache ceiling for the worker.
pub const DEFAULT_CACHE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// Seatbelt profile for the worker (later rules win). The worker is
/// treated as compromisable (it parses model files and runs a large native
/// stack), so the profile removes every way out that it does not need:
/// - no network at all (TCP, UDP, DNS and unix sockets);
/// - no exec of anything but its own interpreter (so no `open`, no
///   `osascript`, no shell), no Apple Events;
/// - no Mach services except the Metal compiler: in particular no
///   LaunchServices (which would launch an unsandboxed app or open a
///   URL for it), pasteboard, Keychain (securityd), WindowServer or
///   preferences daemons;
/// - no signals to, and no inspection of (arguments, environment),
///   other processes; IOKit limited to the GPU and IOSurface clients;
/// - no file contents under the user's home except the runtime, the model
///   store and the worker script (metadata stays readable: Python
///   resolves its own path);
/// - writes only to a private temporary directory.
/// `tests::sandbox_blocks_escapes` pins these against the real runtime.
const SANDBOX_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-exec*)
(allow process-exec (literal (param "PYTHON")))
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
(allow file-read-data (literal (param "HOME")) (subpath (param "RUNTIME")) (subpath (param "MODELS")) (subpath (param "SCRIPT_DIR")))
(deny file-write*)
(allow file-write* (subpath (param "TMP")) (literal "/dev/null") (literal "/dev/dtracehelper"))
"#;

#[derive(Clone, Debug)]
pub struct MlxWorkerConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    /// The model store root (the only model location the worker can read).
    pub models_root: PathBuf,
    /// Run under `sandbox-exec`. Mandatory in production: if it is
    /// unavailable the worker does not start (fail closed). Tests of the
    /// supervisor with a stand-in shell script turn it off.
    pub sandbox: bool,
    /// MLX buffer-cache ceiling (freed buffers above it go back to the
    /// system). `None` keeps MLX's default (unbounded up to its own
    /// heuristics).
    pub cache_limit_bytes: Option<u64>,
    pub hello_timeout: Duration,
    pub load_timeout: Duration,
    pub cancel_grace: Duration,
}

impl MlxWorkerConfig {
    pub fn new(python: PathBuf, script: PathBuf, models_root: PathBuf) -> Self {
        Self {
            python,
            script,
            models_root,
            sandbox: true,
            // Measured (worker_probe, MAI-UI 2B 6-bit): peak footprint
            // 4.9 -> 4.0 GB, same latency, same steady state.
            cache_limit_bytes: Some(DEFAULT_CACHE_LIMIT_BYTES),
            hello_timeout: Duration::from_secs(60),
            load_timeout: Duration::from_secs(240),
            cancel_grace: Duration::from_secs(3),
        }
    }

    /// The product layout: the interpreter shipped inside the signed app
    /// bundle (`Resources/runtime/python`, built by
    /// `scripts/local-model/build-runtime.sh`) and the worker script next
    /// to it (`Resources/workers/mlx/`). Debug builds also accept the
    /// repository's build output. Release builds never run a Python found
    /// in a user-writable location such as the data directory.
    pub fn discover(data_dir: &Path) -> Result<Self, InferenceError> {
        Self::discover_from(
            &runtime_candidates(),
            &worker_script_candidates(),
            data_dir,
        )
    }

    fn discover_from(
        pythons: &[PathBuf],
        scripts: &[PathBuf],
        data_dir: &Path,
    ) -> Result<Self, InferenceError> {
        let python = pythons.iter().find(|p| p.is_file()).ok_or_else(|| {
            InferenceError::RuntimeMissing(
                "the Pegoles Local runtime is missing from this installation; reinstall Pegoles"
                    .into(),
            )
        })?;
        let script = scripts.iter().find(|p| p.is_file()).ok_or_else(|| {
            InferenceError::RuntimeMissing("the local model worker is missing".into())
        })?;
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(InferenceError::RuntimeMissing(
                "macOS sandboxing (sandbox-exec) is unavailable, so Pegoles Local cannot run \
                 its model safely on this Mac"
                    .into(),
            ));
        }
        let canonical = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
        Ok(Self::new(
            canonical(python),
            canonical(script),
            crate::models_dir(data_dir),
        ))
    }

    /// `sandbox-exec` arguments: the profile and its parameters. Values are
    /// passed as `-D` parameters, never spliced into the profile text.
    fn sandbox_args(&self, tmp: &Path) -> Result<Vec<String>, InferenceError> {
        let abs = |p: &Path| -> Result<String, InferenceError> {
            let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            p.to_str()
                .filter(|s| p.is_absolute() && !s.contains('"'))
                .map(str::to_string)
                .ok_or_else(|| {
                    InferenceError::RuntimeMissing(format!("unusable path {}", p.display()))
                })
        };
        let home = std::env::var("HOME").map_err(|_| {
            InferenceError::RuntimeMissing("HOME is not set; cannot sandbox the worker".into())
        })?;
        // bin/python3.12 -> the runtime root.
        let venv = self
            .python
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| InferenceError::RuntimeMissing("bad runtime path".into()))?;
        let script_dir = self
            .script
            .parent()
            .ok_or_else(|| InferenceError::RuntimeMissing("bad worker path".into()))?;
        Ok(vec![
            "-D".into(),
            format!("HOME={}", abs(Path::new(&home))?),
            "-D".into(),
            format!("TMP={}", abs(tmp)?),
            "-D".into(),
            format!("PYTHON={}", abs(&self.python)?),
            "-D".into(),
            format!("RUNTIME={}", abs(venv)?),
            "-D".into(),
            format!("MODELS={}", abs(&self.models_root)?),
            "-D".into(),
            format!("SCRIPT_DIR={}", abs(script_dir)?),
            "-p".into(),
            SANDBOX_PROFILE.into(),
        ])
    }
}

/// The worker's private temporary directory: `pegoles-mlx` inside the
/// per-user Darwin temp dir, owned by this user, mode 0700, never a
/// symlink. It is the only place the sandboxed worker may write.
fn worker_tmp_dir() -> Result<PathBuf, InferenceError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let base = darwin_user_temp_dir().ok_or_else(|| {
        InferenceError::RuntimeMissing("no per-user temporary directory".into())
    })?;
    let dir = base.join("pegoles-mlx");
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(InferenceError::RuntimeMissing(format!(
                "cannot create the worker temp dir: {e}"
            )))
        }
    }
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|e| InferenceError::RuntimeMissing(format!("worker temp dir: {e}")))?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid {
        return Err(InferenceError::RuntimeMissing(
            "the worker temp dir is not a directory owned by this user".into(),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| InferenceError::RuntimeMissing(format!("worker temp dir: {e}")))?;
    }
    Ok(dir)
}

#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: the buffer is valid for `buf.len()` bytes; confstr writes at
    // most that many bytes including the terminating NUL.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if n == 0 || n > buf.len() {
        return None;
    }
    buf.truncate(n - 1);
    let p = PathBuf::from(std::ffi::OsString::from_vec(buf));
    p.is_absolute().then_some(p)
}

#[cfg(not(target_os = "macos"))]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    None
}

/// Interpreter path inside a runtime tree built by `build-runtime.sh`.
const RUNTIME_PYTHON: &str = "runtime/python/bin/python3.12";

fn runtime_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources").join(RUNTIME_PYTHON));
        }
    }
    if cfg!(debug_assertions) {
        out.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/pegoles-runtime/python/bin/python3.12"),
        );
    }
    out
}

fn worker_script_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources/workers/mlx/pegoles_mlx_worker.py"));
        }
    }
    if cfg!(debug_assertions) {
        out.push(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workers/mlx/pegoles_mlx_worker.py"),
        );
    }
    out
}

enum Line {
    Reply(Value),
    Oversized,
    Invalid,
    Eof,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Line>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    pid: i32,
}

impl Proc {
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

#[derive(Clone, Debug, Default)]
pub struct WorkerHello {
    pub worker_version: String,
    pub mlx: String,
    pub mlx_vlm: String,
    pub metal: bool,
}

pub struct MlxWorkerBackend {
    cfg: MlxWorkerConfig,
    proc: Option<Proc>,
    next_id: u64,
    loaded: Option<VerifiedModel>,
    hello: Option<WorkerHello>,
    /// Restarts since creation (crash recovery count).
    pub restarts: u32,
}

impl MlxWorkerBackend {
    pub fn new(cfg: MlxWorkerConfig) -> Self {
        Self {
            cfg,
            proc: None,
            next_id: 1,
            loaded: None,
            hello: None,
            restarts: 0,
        }
    }

    pub fn pid(&self) -> Option<i32> {
        self.proc.as_ref().map(|p| p.pid)
    }

    pub fn hello_info(&self) -> Option<&WorkerHello> {
        self.hello.as_ref()
    }

    pub fn loaded_model(&self) -> Option<&str> {
        self.loaded.as_ref().map(|m| m.spec.id.as_str())
    }

    fn spawn(&mut self) -> Result<(), InferenceError> {
        if !self.cfg.python.exists() {
            return Err(InferenceError::RuntimeMissing(format!(
                "python runtime not found at {}",
                self.cfg.python.display()
            )));
        }
        let mut private_tmp = None;
        let mut cmd = if self.cfg.sandbox {
            if !Path::new(SANDBOX_EXEC).exists() {
                return Err(InferenceError::RuntimeMissing(
                    "sandbox-exec is unavailable; refusing to run the model unsandboxed".into(),
                ));
            }
            let tmp = worker_tmp_dir()?;
            let mut c = Command::new(SANDBOX_EXEC);
            c.args(self.cfg.sandbox_args(&tmp)?).arg(&self.cfg.python);
            private_tmp = Some(tmp);
            c
        } else {
            Command::new(&self.cfg.python)
        };
        cmd.arg("-I") // isolated: no user site, no PYTHON* env, no cwd on path
            .arg(&self.cfg.script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_DATASETS_OFFLINE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONUNBUFFERED", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Set after env_clear (which also drops earlier explicit values).
        match private_tmp {
            Some(tmp) => {
                cmd.env("TMPDIR", tmp);
            }
            None => {
                if let Ok(tmp) = std::env::var("TMPDIR") {
                    cmd.env("TMPDIR", tmp);
                }
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| InferenceError::RuntimeMissing(format!("cannot start worker: {e}")))?;
        let pid = child.id() as i32;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("pegoles-mlx-stdout".into())
            .spawn(move || read_replies(stdout, tx))
            .map_err(|e| InferenceError::WorkerCrashed(e.to_string()))?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let sink = stderr.clone();
        std::thread::Builder::new()
            .name("pegoles-mlx-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr_pipe).lines() {
                    let Ok(line) = line else { break };
                    let mut q = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if q.len() == MAX_STDERR_LINES {
                        q.pop_front();
                    }
                    q.push_back(line.chars().take(MAX_STDERR_LINE_CHARS).collect());
                }
            })
            .map_err(|e| InferenceError::WorkerCrashed(e.to_string()))?;
        self.proc = Some(Proc {
            child,
            stdin,
            rx,
            stderr,
            pid,
        });
        // Request ids are per process: a respawned worker starts over.
        self.next_id = 1;
        self.loaded = None;
        let reply = self.request(json!({"op": "hello"}), self.cfg.hello_timeout, &|| false)?;
        if reply.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
            self.kill();
            return Err(InferenceError::Protocol(
                "worker speaks a different protocol version".into(),
            ));
        }
        let s = |k: &str| {
            reply
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .chars()
                .take(40)
                .collect::<String>()
        };
        self.hello = Some(WorkerHello {
            worker_version: s("worker_version"),
            mlx: s("mlx"),
            mlx_vlm: s("mlx_vlm"),
            metal: reply.get("metal").and_then(Value::as_bool) == Some(true),
        });
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(p) = self.proc.take() {
            p.kill();
        }
        self.loaded = None;
    }

    /// One request/response exchange. Kills the worker on timeout, on a
    /// cancel the worker does not honor within the grace period, and on
    /// any protocol violation.
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
        let proc = self
            .proc
            .as_mut()
            .ok_or_else(|| InferenceError::WorkerCrashed("worker is not running".into()))?;
        if let Err(e) = proc.stdin.write_all(&line).and_then(|_| proc.stdin.flush()) {
            let tail = proc.stderr_tail(4);
            self.kill();
            return Err(InferenceError::WorkerCrashed(format!(
                "write failed: {e}; {tail}"
            )));
        }
        let started = Instant::now();
        let mut cancel_deadline: Option<Instant> = None;
        loop {
            let proc = self.proc.as_mut().expect("present during request");
            match proc.rx.recv_timeout(POLL) {
                Ok(Line::Reply(v)) => {
                    let rid = v.get("id").and_then(Value::as_u64);
                    if rid != Some(id) {
                        // Replies to cancel requests, or stale ones.
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
                    let tail = proc.stderr_tail(6);
                    let status = proc.child.wait().ok();
                    self.proc = None;
                    self.loaded = None;
                    let oom = tail.to_lowercase().contains("memory");
                    let msg = format!(
                        "exit {:?}{}",
                        status.and_then(|s| s.code()),
                        if tail.is_empty() {
                            String::new()
                        } else {
                            format!(": {tail}")
                        }
                    );
                    return Err(if oom {
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
                let proc = self.proc.as_mut().expect("present");
                let _ = writeln!(proc.stdin, "{cancel}");
                let _ = proc.stdin.flush();
                cancel_deadline = Some(Instant::now() + self.cfg.cancel_grace);
            }
            if started.elapsed() > timeout {
                self.kill();
                return Err(InferenceError::Timeout(timeout.as_millis() as u64));
            }
        }
    }

    fn ensure_running(&mut self) -> Result<(), InferenceError> {
        if let Some(p) = self.proc.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(());
            }
            self.kill();
            self.restarts += 1;
        }
        self.spawn()
    }

    fn memory_from(&self, v: &Value) -> BackendMemory {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let proc_mem = self.pid().and_then(hardware::process_memory);
        BackendMemory {
            active_bytes: n("active_bytes"),
            peak_bytes: n("peak_bytes"),
            cache_bytes: n("cache_bytes"),
            process_footprint_bytes: proc_mem.map(|m| m.phys_footprint_bytes),
            process_peak_footprint_bytes: proc_mem.map(|m| m.lifetime_max_phys_footprint_bytes),
        }
    }
}

/// Bounded line reader: a line longer than `MAX_REPLY_BYTES` is never
/// buffered whole; the reader reports it and stops.
fn read_replies(stdout: impl Read, tx: mpsc::Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut buf = Vec::new();
        let n = match (&mut reader)
            .take(MAX_REPLY_BYTES as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) => n,
            Err(_) => {
                let _ = tx.send(Line::Eof);
                return;
            }
        };
        if n == 0 {
            let _ = tx.send(Line::Eof);
            return;
        }
        if buf.len() > MAX_REPLY_BYTES {
            let _ = tx.send(Line::Oversized);
            return;
        }
        let line = match serde_json::from_slice::<Value>(&buf) {
            Ok(v)
                if v.is_object()
                    && v.get("v").and_then(Value::as_u64) == Some(PROTOCOL_VERSION) =>
            {
                Line::Reply(v)
            }
            _ => Line::Invalid,
        };
        let stop = matches!(line, Line::Invalid);
        if tx.send(line).is_err() || stop {
            return;
        }
    }
}

fn wire_messages(messages: &[ChatMessage]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let content: Vec<Value> = m
                    .parts
                    .iter()
                    .map(|p| match p {
                        Part::Text(t) => json!({"type": "text", "text": t}),
                        Part::Image => json!({"type": "image"}),
                    })
                    .collect();
                json!({"role": role, "content": content})
            })
            .collect(),
    )
}

impl InferenceBackend for MlxWorkerBackend {
    fn info(&self) -> BackendInfo {
        let h = self.hello.clone().unwrap_or_default();
        BackendInfo {
            backend: "mlx".into(),
            runtime_version: format!("mlx {} / mlx-vlm {}", h.mlx, h.mlx_vlm),
            accelerator: if h.metal { "metal" } else { "unknown" }.into(),
        }
    }

    fn ensure_loaded(
        &mut self,
        model: &VerifiedModel,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<LoadReport>, InferenceError> {
        self.ensure_running()?;
        if self.loaded.as_ref() == Some(model) {
            return Ok(None);
        }
        let dir = model
            .dir
            .to_str()
            .ok_or_else(|| InferenceError::BadRequest("model path is not UTF-8".into()))?;
        let mut load = json!({"op": "load", "model_dir": dir});
        if let Some(limit) = self.cfg.cache_limit_bytes {
            load["cache_limit_bytes"] = json!(limit);
        }
        let reply = self.request(load, self.cfg.load_timeout, cancelled);
        let reply = match reply {
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
            verify_ms: model.verify_ms,
            memory: self.memory_from(&reply),
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
        let finish = reply
            .get("finish")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(20)
            .collect::<String>();
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
            memory: self.memory_from(&reply),
        })
    }

    fn loaded_model_id(&self) -> Option<&str> {
        self.loaded_model()
    }

    fn process_id(&self) -> Option<i32> {
        self.pid()
    }

    fn memory(&mut self) -> Result<BackendMemory, InferenceError> {
        if self.proc.is_none() {
            return Ok(BackendMemory::default());
        }
        let reply = self.request(json!({"op": "stats"}), Duration::from_secs(10), &|| false)?;
        Ok(self.memory_from(&reply))
    }

    fn unload(&mut self) -> Result<(), InferenceError> {
        if self.proc.is_some() {
            self.request(json!({"op": "unload"}), Duration::from_secs(30), &|| false)?;
        }
        self.loaded = None;
        Ok(())
    }

    fn shutdown(&mut self) {
        if self.proc.is_some() {
            let _ = self.request(json!({"op": "shutdown"}), Duration::from_secs(3), &|| false);
            if let Some(mut p) = self.proc.take() {
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    if matches!(p.child.try_wait(), Ok(Some(_))) {
                        break;
                    }
                    std::thread::sleep(POLL);
                }
                p.kill();
            }
        }
        self.loaded = None;
    }
}

impl Drop for MlxWorkerBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in "worker" (shell script) exercises the supervisor:
    /// crashes, garbage, oversized replies and hangs are contained.
    fn fake_worker(tmp: &Path, body: &str) -> MlxWorkerConfig {
        let script = tmp.join("worker.sh");
        std::fs::write(&script, body).unwrap();
        let mut cfg = MlxWorkerConfig::new(PathBuf::from("/bin/sh"), script, tmp.to_path_buf());
        cfg.sandbox = false;
        cfg.hello_timeout = Duration::from_secs(5);
        cfg.cancel_grace = Duration::from_millis(300);
        cfg
    }

    // `/bin/sh -I` is not valid; the fake uses a wrapper that ignores it.
    fn sh_cfg(tmp: &Path, body: &str) -> MlxWorkerConfig {
        let wrapper = tmp.join("python");
        std::fs::write(&wrapper, "#!/bin/sh\nshift\nexec /bin/sh \"$@\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = fake_worker(tmp, body);
        cfg.python = wrapper;
        cfg
    }

    const HELLO: &str = r#"read line; echo '{"v":1,"id":1,"ok":true,"protocol":1,"worker_version":"t","mlx":"x","mlx_vlm":"y","metal":true}'"#;

    #[test]
    #[cfg(unix)]
    fn handshake_then_crash_is_reported_and_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(tmp.path(), &format!("{HELLO}\necho boom >&2\nexit 9\n"));
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        assert!(w.hello_info().unwrap().metal);
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(
            matches!(err, InferenceError::WorkerCrashed(ref m) if m.contains("boom")),
            "{err:?}"
        );
        assert!(w.pid().is_none());
        // Next use respawns.
        w.ensure_running().unwrap();
        assert!(w.pid().is_some());
    }

    #[test]
    #[cfg(unix)]
    fn garbage_and_oversized_replies_kill_the_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nread line\necho 'not json'\nsleep 5\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(matches!(err, InferenceError::Protocol(_)));
        assert!(w.pid().is_none());

        let big = "x".repeat(MAX_REPLY_BYTES + 10);
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nread line\nprintf '{{\"v\":1,\"id\":2,\"ok\":true,\"pad\":\"{big}\"}}\\n'\nsleep 5\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(matches!(err, InferenceError::Protocol(_)), "{err:?}");
    }

    #[test]
    #[cfg(unix)]
    fn hang_times_out_and_unhonored_cancel_kills() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nwhile read line; do :; done\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_millis(300), &|| {
                false
            })
            .unwrap_err();
        assert_eq!(err, InferenceError::Timeout(300));
        assert!(t.elapsed() < Duration::from_secs(3));
        assert!(w.pid().is_none());

        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(30), &|| true)
            .unwrap_err();
        assert_eq!(err, InferenceError::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[cfg(unix)]
    fn worker_errors_map_to_typed_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!(
                "{HELLO}\nread line\necho '{{\"v\":1,\"id\":2,\"ok\":false,\"error\":{{\"kind\":\"out_of_memory\",\"message\":\"metal oom\"}}}}'\nsleep 5\n"
            ),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "generate"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert_eq!(err, InferenceError::OutOfMemory("metal oom".into()));
    }

    /// Escape attempts from inside the real sandbox profile, run with the
    /// real runtime built by `scripts/local-model/build-runtime.sh` (skipped
    /// when it has not been built, e.g. in CI). Every probe is first run
    /// unsandboxed as a control (it must work there, so "blocked" really
    /// means the sandbox), then under the worker profile, where it must
    /// fail; Metal and the private temp dir must keep working.
    const ESCAPE_PROBES: &str = r#"
import ctypes, json, os, socket, subprocess, sys
parent, port, home_dir, outside, private = sys.argv[1:6]
parent, port = int(parent), int(port)
res = {}
def probe(name, fn):
    try:
        fn()
        res[name] = "allowed"
    except Exception:
        res[name] = "blocked"
libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
def lookup(service):
    port = ctypes.c_uint32(0)
    bp = ctypes.c_uint32.in_dll(libc, "bootstrap_port")
    if libc.bootstrap_look_up(bp, service.encode(), ctypes.byref(port)) != 0:
        raise OSError("lookup failed")
def pidpath(pid):
    buf = ctypes.create_string_buffer(4096)
    if libc.proc_pidpath(pid, buf, 4096) <= 0:
        raise OSError("proc_pidpath failed")
def write(path):
    with open(path, "w") as f:
        f.write("x")
    os.unlink(path)
def metal():
    import mlx.core as mx
    x = (mx.arange(4096, dtype=mx.float32) * 3 + 1).sum()
    mx.eval(x)
    assert mx.metal.is_available() and x.item() > 0
probe("exec", lambda: subprocess.run(["/usr/bin/true"], check=True))
probe("tcp_loopback", lambda: socket.create_connection(("127.0.0.1", port), timeout=3).close())
probe("dns_socket", lambda: socket.socket(socket.AF_UNIX).connect("/var/run/mDNSResponder"))
probe("read_home", lambda: open(os.path.join(home_dir, "canary"), "rb").read())
probe("write_home", lambda: write(os.path.join(home_dir, "written")))
probe("write_shared_tmp", lambda: write(os.path.join(outside, "pegoles-sbx-probe-%d" % os.getpid())))
probe("launchservices", lambda: lookup("com.apple.coreservices.launchservicesd"))
probe("pasteboard", lambda: lookup("com.apple.pasteboard.1"))
probe("securityd", lambda: lookup("com.apple.SecurityServer"))
probe("securityd_xpc", lambda: lookup("com.apple.securityd.xpc"))
probe("lsd_open", lambda: lookup("com.apple.lsd.open"))
probe("appleevents", lambda: lookup("com.apple.coreservices.appleevents"))
probe("windowserver", lambda: lookup("com.apple.windowserver.active"))
probe("cfprefsd", lambda: lookup("com.apple.cfprefsd.daemon"))
probe("signal_parent", lambda: os.kill(parent, 0))
probe("inspect_parent", lambda: pidpath(parent))
probe("metal", metal)
probe("write_private_tmp", lambda: write(os.path.join(private, "ok-%d" % os.getpid())))
print(json.dumps(res))
"#;

    const ESCAPES: [&str; 16] = [
        "exec",
        "tcp_loopback",
        "dns_socket",
        "read_home",
        "write_home",
        "write_shared_tmp",
        "launchservices",
        "pasteboard",
        "securityd",
        "securityd_xpc",
        "lsd_open",
        "appleevents",
        "windowserver",
        "cfprefsd",
        "signal_parent",
        "inspect_parent",
    ];

    #[test]
    fn sandbox_blocks_escapes() {
        let python = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/pegoles-runtime/python/bin/python3.12");
        if !python.is_file() || !Path::new(SANDBOX_EXEC).exists() {
            eprintln!("skipped: build the runtime with scripts/local-model/build-runtime.sh");
            return;
        }
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        // A canary directory under HOME (outside every allowed subpath).
        let home_dir = tempfile::tempdir_in(&home).unwrap();
        std::fs::write(home_dir.path().join("canary"), "secret").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let models = tempfile::tempdir().unwrap();
        let script = models.path().join("worker.py");
        std::fs::write(&script, "").unwrap();
        let cfg = MlxWorkerConfig::new(python.clone(), script, models.path().to_path_buf());
        let tmp = worker_tmp_dir().unwrap();
        let outside = tmp.parent().unwrap().to_path_buf();
        let run = |sandboxed: bool| -> Value {
            let mut cmd = if sandboxed {
                let mut c = Command::new(SANDBOX_EXEC);
                c.args(cfg.sandbox_args(&tmp).unwrap()).arg(&python);
                c
            } else {
                Command::new(&python)
            };
            let out = cmd
                .args(["-I", "-c", ESCAPE_PROBES])
                .arg(std::process::id().to_string())
                .arg(&port)
                .arg(home_dir.path())
                .arg(&outside)
                .arg(&tmp)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TMPDIR", &tmp)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "probe script failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            serde_json::from_slice(&out.stdout).unwrap()
        };
        let control = run(false);
        let boxed = run(true);
        for probe in ESCAPES {
            assert_eq!(control[probe], "allowed", "control {probe}: {control}");
            assert_eq!(boxed[probe], "blocked", "{probe} escaped the sandbox: {boxed}");
        }
        assert_eq!(boxed["metal"], "allowed", "{boxed}");
        assert_eq!(boxed["write_private_tmp"], "allowed", "{boxed}");
        assert!(!home_dir.path().join("written").exists());
    }

    #[test]
    fn missing_runtime_is_a_clear_error() {
        let cfg = MlxWorkerConfig::new(
            PathBuf::from("/nonexistent/python"),
            PathBuf::from("x"),
            PathBuf::from("/nonexistent"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        assert!(matches!(w.spawn(), Err(InferenceError::RuntimeMissing(_))));
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("worker.py");
        std::fs::write(&script, "").unwrap();
        // No interpreter at any candidate: a clear error, never a fallback.
        assert!(matches!(
            MlxWorkerConfig::discover_from(
                &[tmp.path().join("runtime/python/bin/python3.12")],
                std::slice::from_ref(&script),
                tmp.path(),
            ),
            Err(InferenceError::RuntimeMissing(_))
        ));
        // An interpreter but no worker script: also refused.
        let python = tmp.path().join("python3.12");
        std::fs::write(&python, "").unwrap();
        assert!(matches!(
            MlxWorkerConfig::discover_from(
                std::slice::from_ref(&python),
                &[tmp.path().join("missing.py")],
                tmp.path(),
            ),
            Err(InferenceError::RuntimeMissing(_))
        ));
    }
}

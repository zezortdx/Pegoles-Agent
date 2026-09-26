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

/// Seatbelt profile for the worker (later rules win). No network at all
/// (TCP and DNS verified denied). No file contents under the user's home
/// are readable except the runtime, the model store and the worker
/// script (metadata stays readable: Python resolves its own path);
/// nothing is writable except the per-user temporary/cache area, where
/// Metal keeps its shader cache.
const SANDBOX_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny file-read-data (subpath (param "HOME")))
(allow file-read-data (literal (param "HOME")) (subpath (param "RUNTIME")) (subpath (param "MODELS")) (subpath (param "SCRIPT_DIR")))
(deny file-write*)
(allow file-write* (subpath "/private/var/folders") (literal "/dev/null") (literal "/dev/dtracehelper"))
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

    /// The product layout: the runtime venv under the Pegoles data dir,
    /// the worker script shipped next to the executable (app bundle
    /// `Resources/workers/mlx/`), or the repository copy in debug builds.
    pub fn discover(data_dir: &Path) -> Result<Self, InferenceError> {
        let python = data_dir.join("runtime/mlx-venv/bin/python");
        if !python.exists() {
            return Err(InferenceError::RuntimeMissing(
                "the Pegoles Local runtime is not set up on this Mac".into(),
            ));
        }
        let script = worker_script_candidates()
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                InferenceError::RuntimeMissing("the local model worker is missing".into())
            })?;
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(InferenceError::RuntimeMissing(
                "macOS sandboxing (sandbox-exec) is unavailable, so Pegoles Local cannot run \
                 its model safely on this Mac"
                    .into(),
            ));
        }
        Ok(Self::new(python, script, crate::models_dir(data_dir)))
    }

    /// `sandbox-exec` arguments: the profile and its parameters.
    fn sandbox_args(&self) -> Result<Vec<String>, InferenceError> {
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
        // bin/python -> the venv root.
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

fn worker_script_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources/workers/mlx/pegoles_mlx_worker.py"));
            out.push(dir.join("workers/mlx/pegoles_mlx_worker.py"));
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
        let mut cmd = if self.cfg.sandbox {
            if !Path::new(SANDBOX_EXEC).exists() {
                return Err(InferenceError::RuntimeMissing(
                    "sandbox-exec is unavailable; refusing to run the model unsandboxed".into(),
                ));
            }
            let mut c = Command::new(SANDBOX_EXEC);
            c.args(self.cfg.sandbox_args()?).arg(&self.cfg.python);
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
        if let Ok(tmp) = std::env::var("TMPDIR") {
            cmd.env("TMPDIR", tmp);
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
        assert!(matches!(
            MlxWorkerConfig::discover(tmp.path()),
            Err(InferenceError::RuntimeMissing(_))
        ));
    }
}

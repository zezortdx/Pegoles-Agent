//! pegoles-llm-worker — Pegoles Local's llama.cpp worker.
//!
//! A persistent process that keeps one vision-language model loaded and
//! answers generation requests. It is an inference engine only:
//!
//! * It never executes, evaluates or interprets model output: text goes
//!   back to the Pegoles runtime, which parses it strictly into typed
//!   actions that pass through the deterministic policy.
//! * It never opens a network connection (the host runs it with no network
//!   rights: an AppContainer without capabilities on Windows,
//!   `sandbox-exec` on macOS) and has no code for one.
//! * It loads only GGUF files from a model folder the host verified
//!   against pinned digests; GGUF carries weights and metadata, no code.
//!
//! Protocol v1, identical to the MLX worker's: JSON lines on stdin and
//! stdout (stdout is moved off fd 1 first, so a library printing cannot
//! corrupt it); a `cancel` request stops a generation between tokens.

mod engine;
mod prep;
mod protocol;

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use llama_cpp_2::llama_backend::LlamaBackend;
use serde_json::{json, Value};

use protocol::{fail, reply, BadRequest, MAX_IMAGES, MAX_LINE_BYTES, PROTOCOL_VERSION};

/// The reply channel: a private copy of the original stdout. Fd 1 then
/// points at stderr, so stray output from llama.cpp lands in diagnostics.
fn private_stdout() -> std::fs::File {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        // SAFETY: dup/dup2 on the standard descriptors.
        unsafe {
            let proto = libc::dup(1);
            libc::dup2(2, 1);
            std::fs::File::from_raw_fd(proto)
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        // SAFETY: CRT descriptor calls on the standard descriptors; the OS
        // handle of the duplicate belongs to us from here on.
        unsafe {
            let proto = libc::dup(1);
            let handle = libc::get_osfhandle(proto);
            libc::dup2(2, 1);
            std::fs::File::from_raw_handle(handle as _)
        }
    }
}

struct Out(Mutex<std::fs::File>);

impl Out {
    fn send(&self, line: String) {
        if let Ok(mut out) = self.0.lock() {
            let _ = out.write_all(line.as_bytes());
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
}

fn log(message: &str) {
    let text: String = message.chars().take(2000).collect();
    let _ = writeln!(std::io::stderr(), "[pegoles-llm-worker] {text}");
}

struct Current {
    id: Option<i64>,
    cancel: Arc<AtomicBool>,
}

fn reader(out: Arc<Out>, current: Arc<Mutex<Current>>, jobs: mpsc::Sender<Option<Value>>) {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    loop {
        let mut line = Vec::new();
        let n = (&mut input)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .unwrap_or_default();
        if n == 0 {
            let _ = jobs.send(None);
            return;
        }
        if line.len() > MAX_LINE_BYTES && !line.ends_with(b"\n") {
            // Drain the rest of the oversized line, then report it.
            let mut rest = Vec::new();
            while !rest.ends_with(b"\n") {
                rest.clear();
                if (&mut input)
                    .take(MAX_LINE_BYTES as u64)
                    .read_until(b'\n', &mut rest)
                    .unwrap_or(0)
                    == 0
                {
                    let _ = jobs.send(None);
                    return;
                }
            }
            out.send(fail(&Value::Null, "bad_request", "request line too long"));
            continue;
        }
        let Ok(request) = serde_json::from_slice::<Value>(&line) else {
            out.send(fail(
                &Value::Null,
                "bad_request",
                "request is not valid JSON",
            ));
            continue;
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if request.get("v").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
            out.send(fail(&id, "bad_request", "unsupported protocol version"));
            continue;
        }
        if !id.is_i64() {
            out.send(fail(
                &Value::Null,
                "bad_request",
                "request id must be an integer",
            ));
            continue;
        }
        if request.get("op").and_then(Value::as_str) == Some("cancel") {
            if let Some(target) = request.get("target").and_then(Value::as_i64) {
                if let Ok(cur) = current.lock() {
                    if cur.id == Some(target) {
                        cur.cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
            out.send(reply(&id, json!({})));
            continue;
        }
        if jobs.send(Some(request)).is_err() {
            return;
        }
    }
}

struct Worker {
    backend: LlamaBackend,
    loaded: Option<engine::Loaded>,
    gpu: bool,
    accelerator: engine::Accelerator,
}

impl Worker {
    fn hello(&self, id: &Value) -> String {
        reply(
            id,
            json!({
                "worker": "pegoles-llama",
                "worker_version": protocol::WORKER_VERSION,
                "protocol": PROTOCOL_VERSION,
                "runtime": "llama.cpp (llama-cpp-2 0.1.157)",
                "backend": self.accelerator.backend,
                "device": self.accelerator.device,
                "gpu": self.gpu,
                "metal": self.accelerator.backend.eq_ignore_ascii_case("metal"),
            }),
        )
    }

    fn load(&mut self, id: &Value, req: &Value) -> Result<String, BadRequest> {
        let dir = req
            .get("model_dir")
            .and_then(Value::as_str)
            .filter(|d| std::path::Path::new(d).is_absolute())
            .ok_or_else(|| BadRequest("model_dir must be an absolute path".into()))?;
        let file = |key: &str| {
            req.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| BadRequest(format!("{key} is required")))
        };
        let (model_file, projector_file) = (file("model_file")?, file("projector_file")?);
        self.loaded = None;
        let started = std::time::Instant::now();
        let loaded = engine::load(&self.backend, dir, model_file, projector_file, self.gpu)
            .map_err(BadRequest)?;
        self.loaded = Some(loaded);
        let mut fields =
            json!({"load_ms": (started.elapsed().as_secs_f64() * 10000.0).round() / 10.0});
        merge(&mut fields, engine::memory_fields());
        Ok(reply(id, fields))
    }

    fn generate(
        &mut self,
        id: &Value,
        req: &Value,
        cancel: &AtomicBool,
    ) -> Result<String, BadRequest> {
        let loaded = self
            .loaded
            .as_mut()
            .ok_or_else(|| BadRequest("no model is loaded".into()))?;
        let (messages, slots) =
            protocol::sanitize_messages(req.get("messages").unwrap_or(&Value::Null))?;
        let images_b64: Vec<&str> = match req.get("images") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => v
                .as_array()
                .filter(|a| a.len() <= MAX_IMAGES)
                .ok_or_else(|| BadRequest("at most two images per request".into()))?
                .iter()
                .map(|i| {
                    i.as_str()
                        .ok_or_else(|| BadRequest("images must be base64 strings".into()))
                })
                .collect::<Result<_, _>>()?,
        };
        if slots != images_b64.len() {
            return Err(BadRequest("image placeholders do not match images".into()));
        }
        let prep = protocol::parse_prep(req.get("image_prep"), images_b64.len())?;
        let (max_tokens, temperature) = protocol::parse_sampling(req)?;
        let t0 = std::time::Instant::now();
        let images = images_b64
            .iter()
            .zip(&prep)
            .map(|(b, p)| prep::decode(b, p))
            .collect::<Result<Vec<_>, _>>()?;
        let decode_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let prompt = protocol::chatml(&messages);
        let out = engine::generate(loaded, &prompt, &images, max_tokens, temperature, cancel)
            .map_err(|e| BadRequest(format!("inference failed: {e}")))?;
        let total = t0.elapsed().as_secs_f64() * 1000.0;
        let secs = |ms: f64| (ms / 1000.0).max(1e-6);
        let mut fields = json!({
            "text": out.text,
            "finish": out.finish,
            "prompt_tokens": out.prompt_tokens,
            "generation_tokens": out.generation_tokens,
            "prompt_tps": ((out.prompt_tokens as f64 / secs(out.image_ms)) * 100.0).round() / 100.0,
            "generation_tps": ((out.generation_tokens as f64 / secs(out.generate_ms)) * 100.0).round() / 100.0,
            "image_sizes": images.iter().map(|i| json!([i.width(), i.height()])).collect::<Vec<_>>(),
            "timings_ms": {
                "image": ((decode_ms + out.image_ms) * 10.0).round() / 10.0,
                "first_token": (out.first_token_ms * 10.0).round() / 10.0,
                "generate": (out.generate_ms * 10.0).round() / 10.0,
                "total": (total * 10.0).round() / 10.0,
            },
        });
        merge(&mut fields, engine::memory_fields());
        Ok(reply(id, fields))
    }
}

fn merge(target: &mut Value, extra: Value) {
    if let (Some(t), Value::Object(e)) = (target.as_object_mut(), extra) {
        t.extend(e);
    }
}

fn main() {
    let out = Arc::new(Out(Mutex::new(private_stdout())));
    #[cfg(windows)]
    {
        // GPU/CPU backend modules only from our own folder, never the
        // working directory or an environment override.
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        {
            llama_cpp_2::llama_backend::load_backends_from_path(&dir);
        }
    }
    let backend = match LlamaBackend::init() {
        Ok(b) => b,
        Err(e) => {
            out.send(fail(
                &Value::Null,
                "runtime_missing",
                &format!("llama.cpp could not start: {e}"),
            ));
            std::process::exit(3);
        }
    };
    let accelerator = engine::accelerator(std::env::var("PEGOLES_LLM_CPU").as_deref() == Ok("1"));
    let gpu = accelerator.gpu;
    log(&format!(
        "backend {} ({}) gpu={gpu}",
        accelerator.backend, accelerator.device
    ));
    let mut worker = Worker {
        backend,
        loaded: None,
        gpu,
        accelerator,
    };
    let current = Arc::new(Mutex::new(Current {
        id: None,
        cancel: Arc::new(AtomicBool::new(false)),
    }));
    let (tx, rx) = mpsc::channel();
    {
        let out = out.clone();
        let current = current.clone();
        std::thread::spawn(move || reader(out, current, tx));
    }
    while let Ok(Some(request)) = rx.recv() {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let op = request.get("op").and_then(Value::as_str).unwrap_or("");
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut cur) = current.lock() {
            *cur = Current {
                id: id.as_i64(),
                cancel: cancel.clone(),
            };
        }
        let result = match op {
            "hello" => Ok(worker.hello(&id)),
            "load" => worker.load(&id, &request),
            "unload" => {
                worker.loaded = None;
                Ok(reply(&id, engine::memory_fields()))
            }
            "stats" => {
                let mut fields = json!({"loaded": worker.loaded.is_some()});
                merge(&mut fields, engine::memory_fields());
                Ok(reply(&id, fields))
            }
            "generate" => worker.generate(&id, &request, &cancel),
            "shutdown" => {
                out.send(reply(&id, json!({})));
                return;
            }
            _ => Err(BadRequest("unknown op".into())),
        };
        if let Ok(mut cur) = current.lock() {
            cur.id = None;
        }
        match result {
            Ok(line) => out.send(line),
            Err(BadRequest(message)) => {
                let kind = if message.to_lowercase().contains("memory") {
                    "out_of_memory"
                } else {
                    "bad_request"
                };
                out.send(fail(&id, kind, &message));
            }
        }
    }
}

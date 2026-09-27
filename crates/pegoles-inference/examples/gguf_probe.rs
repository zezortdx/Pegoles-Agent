//! Real generations through the llama.cpp worker, confined exactly as in
//! the app (AppContainer + job object on Windows, sandbox-exec on macOS),
//! on a real guest screenshot. Prints what the worker ran on, the load
//! time, per-generation timings and memory, and the model's answer.
//!
//! ```sh
//! cargo run --release -p pegoles-inference --example gguf_probe -- \
//!   <pegoles-llm-worker> [cpu]
//! ```
//! The model (`mai-ui-2b-q8-gguf`) must be installed in the data folder
//! (`models install mai-ui-2b-q8-gguf`; `PEGOLES_DATA_DIR` overrides it).
//! The screenshot is `tests/fixtures/guest-screen-windows.png`: the
//! x64 guest's screen, captured through Hyper-V by CI's agent E2E, with
//! the red block that run painted in its terminal.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pegoles_inference::{
    hardware, models_dir, Catalog, ChatMessage, GenerateRequest, ImageInput, InferenceBackend,
    LlamaWorkerBackend, LlamaWorkerConfig, ModelStore, Part, Role,
};

const MODEL: &str = "mai-ui-2b-q8-gguf";
const RUNS: usize = 3;

fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PEGOLES_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Pegoles");
    }
    PathBuf::from(std::env::var("HOME").expect("HOME")).join("Library/Application Support/Pegoles")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let worker = PathBuf::from(args.first().expect("usage: gguf_probe <worker> [cpu]"));
    let cpu = args.iter().any(|a| a == "cpu");
    let spec = Catalog::builtin()
        .get(MODEL)
        .cloned()
        .expect("catalog entry");
    let verified = ModelStore::new(models_dir(&data_dir()))
        .verify(&spec)
        .expect("model installed and verified");
    let mut cfg = LlamaWorkerConfig::new(worker, models_dir(&data_dir()));
    cfg.cpu_only = cpu;
    let mut backend = LlamaWorkerBackend::new(cfg);
    let load = backend
        .ensure_loaded(&verified, &|| false)
        .expect("load")
        .expect("first load reports");
    println!("backend: {:?}", backend.info());
    println!("hello: {:?}", backend.hello_info());
    println!("load {:.0} ms, verify {} ms", load.load_ms, load.verify_ms);

    let png = Arc::new(
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/guest-screen-windows.png"),
        )
        .expect("fixture"),
    );
    let request = GenerateRequest {
        messages: vec![
            ChatMessage::text(
                Role::System,
                "You are a GUI agent. Answer with one line of JSON.",
            ),
            ChatMessage {
                role: Role::User,
                parts: vec![
                    Part::Image,
                    Part::Text(
                        "Where is the red block on this screen? Reply {\"x\": <0-999>, \"y\": <0-999>} for its center."
                            .into(),
                    ),
                ],
            },
        ],
        images: vec![ImageInput {
            png,
            crop: None,
            // The planner's model-input size for a 1440x900 screen.
            resize: Some((1440, 896)),
        }],
        max_tokens: 48,
        temperature: 0.0,
        timeout: Duration::from_secs(900),
    };
    for run in 0..RUNS {
        let started = Instant::now();
        let reply = backend.generate(&request, &|| false).expect("generate");
        let peak = backend
            .process_id()
            .and_then(hardware::process_memory)
            .map(|m| m.lifetime_max_phys_footprint_bytes);
        println!(
            "run {run}: wall {:.0} ms, image+prefill {:.0} ms, generation {:.0} ms, prompt {} tokens, generated {} ({:.1} tok/s), worker peak {:?} bytes, finish {}",
            started.elapsed().as_secs_f64() * 1000.0,
            reply.timings.image_ms,
            reply.timings.generate_ms,
            reply.prompt_tokens,
            reply.generation_tokens,
            reply.generation_tps,
            peak,
            reply.finish,
        );
        if run == 0 {
            println!("answer: {}", reply.text.trim());
        }
    }
    backend.shutdown();
}

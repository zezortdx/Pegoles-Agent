//! Measure the MLX worker alone (no VM): model-input size and MLX cache
//! limit against latency and memory, on a real guest screenshot.
//!
//! ```sh
//! cargo run --release -p pegoles-inference --example worker_probe -- \
//!   <model-id> <screenshot.png> <long-side>[,<long-side>…] [cache-limit-MiB]
//! ```
//! Prints one JSON line per size: prompt tokens, first-token / total
//! latency (median of 4 after a warm-up), MLX peak, and the worker
//! process footprint sampled every 50 ms (steady and peak).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pegoles_inference::{
    hardware, models_dir, Catalog, ChatMessage, GenerateRequest, ImageInput, InferenceBackend,
    MlxWorkerBackend, MlxWorkerConfig, ModelStore, Part, Role,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data =
        PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support/Pegoles");
    let spec = Catalog::builtin().get(&args[0]).cloned().expect("model id");
    let png = Arc::new(std::fs::read(&args[1]).expect("png"));
    let sizes: Vec<u32> = args[2].split(',').map(|v| v.parse().unwrap()).collect();
    let cache_mib: Option<u64> = args.get(3).map(|v| v.parse().unwrap());
    let verified = ModelStore::new(models_dir(&data))
        .verify(&spec)
        .expect("verified model");
    // Release builds only look next to the executable: point at the repo copy.
    let mut cfg = MlxWorkerConfig::new(
        data.join("runtime/mlx-venv/bin/python"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workers/mlx/pegoles_mlx_worker.py"),
        models_dir(&data),
    );
    cfg.cache_limit_bytes = cache_mib.map(|m| m << 20);
    let mut w = MlxWorkerBackend::new(cfg);
    let load = w
        .ensure_loaded(&verified, &|| false)
        .expect("load")
        .unwrap();
    let pid = w.pid().unwrap();
    let peak = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (peak, stop) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(m) = hardware::process_memory(pid) {
                    peak.fetch_max(m.phys_footprint_bytes, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }
    let loaded_fp = hardware::process_memory(pid).map_or(0, |m| m.phys_footprint_bytes);
    println!("{{\"model\":\"{}\",\"load_ms\":{},\"footprint_after_load\":{loaded_fp},\"cache_limit_mib\":{cache_mib:?}}}", spec.id, load.load_ms);
    let system = if spec.family == pegoles_inference::ModelFamily::MaiUi {
        "You are a GUI agent. You are given a task and your action history, with screenshots. You need to perform the next action to complete the task. Reply with <thinking></thinking> then one <tool_call>{\"name\": \"mobile_use\", \"arguments\": {\"action\": \"click\", \"coordinate\": [x, y]}}</tool_call>."
    } else {
        "You are a helpful assistant. Reply with one <tool_call>{\"name\": \"computer_use\", \"arguments\": {\"action\": \"left_click\", \"coordinate\": [x, y]}}</tool_call>; coordinates are 0-1000 relative."
    };
    for &side in &sizes {
        let (iw, ih) = (1440u32, 900u32);
        let scale = (side as f64 / iw as f64).min(1.0);
        let snap = |v: u32| (((v as f64 * scale) / 32.0).round() as u32 * 32).max(32);
        let req = GenerateRequest {
            messages: vec![
                ChatMessage::text(Role::System, system),
                ChatMessage::text(Role::User, "Task: Close the terminal window."),
                ChatMessage {
                    role: Role::User,
                    parts: vec![Part::Image],
                },
            ],
            images: vec![ImageInput {
                png: png.clone(),
                crop: None,
                resize: Some((snap(iw), snap(ih))),
            }],
            max_tokens: 200,
            temperature: 0.0,
            timeout: Duration::from_secs(120),
        };
        peak.store(0, Ordering::Relaxed);
        let mut runs = Vec::new();
        for i in 0..5 {
            let r = w.generate(&req, &|| false).expect("generate");
            if i > 0 {
                runs.push(r);
            }
        }
        let mut first: Vec<f64> = runs.iter().map(|r| r.timings.first_token_ms).collect();
        let mut wall: Vec<f64> = runs.iter().map(|r| r.timings.wall_ms).collect();
        first.sort_by(|a, b| a.partial_cmp(b).unwrap());
        wall.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let steady = hardware::process_memory(pid).map_or(0, |m| m.phys_footprint_bytes);
        let last = runs.last().unwrap();
        println!(
            "{{\"long_side\":{side},\"image\":{:?},\"prompt_tokens\":{},\"first_token_ms\":{:.0},\"wall_ms\":{:.0},\"mlx_peak\":{},\"footprint_steady\":{steady},\"footprint_peak\":{},\"text\":{:?}}}",
            last.image_sizes, last.prompt_tokens, first[first.len() / 2], wall[wall.len() / 2],
            last.memory.peak_bytes, peak.load(Ordering::Relaxed),
            last.text.chars().rev().take(90).collect::<String>().chars().rev().collect::<String>()
        );
    }
    stop.store(true, Ordering::Relaxed);
    w.shutdown();
}

//! Release soak on real hardware: repeated computer lifecycles through the
//! product path (create/boot, observe, type, reset, destroy) and repeated
//! local-model inference with forced worker restarts, while host resources
//! are sampled. Fails on growth or anything left behind.
//!
//! ```sh
//! cargo build --release -p pegoles-agent --example release_soak
//! cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
//! ./target/release/examples/release_soak --cycles 12 --observes 100 --inferences 60
//! ```
//!
//! Checks (exit 2 if any fails):
//! - after every destroy: no Pegoles VM process, no helper, the computers
//!   directory holds no computer;
//! - this process's memory and descriptors do not grow across cycles;
//! - every observe and typed action in a cycle succeeds (a transient guest
//!   capture timeout is reported and retried once);
//! - inference: the worker is killed every 20 requests and must come back;
//!   after shutdown no worker process remains; its temp dir stays small.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pegoles_agent::{AgentComputer, CoreAccess, CoreComputer};
use pegoles_computer::platform::BackendKind;
use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, TaskManager};
use pegoles_inference::{
    hardware, models_dir, Catalog, ChatMessage, GenerateRequest, ImageInput, InferenceBackend,
    MlxWorkerBackend, MlxWorkerConfig, ModelStore, Part, Role,
};
use pegoles_protocol::{ComputerAction, TaskStatus};

struct Core {
    registry: ComputerRegistry,
    tasks: TaskManager,
}

#[derive(Clone)]
struct Shared(Arc<Mutex<Core>>);

impl CoreAccess for Shared {
    fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let core = &mut *guard;
        f(&mut core.registry, &mut core.tasks)
    }
}

fn arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn pids(pattern: &str, exact: bool) -> BTreeSet<i32> {
    let flag = if exact { "-x" } else { "-f" };
    std::process::Command::new("/usr/bin/pgrep")
        .args([flag, pattern])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn own_fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(0)
}

fn footprint_mb(pid: i32) -> u64 {
    hardware::process_memory(pid).map_or(0, |m| m.phys_footprint_bytes >> 20)
}

fn dir_kb(path: &Path) -> u64 {
    std::process::Command::new("/usr/bin/du")
        .arg("-sk")
        .arg(path)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn computers_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0)
}

fn main() {
    let cycles: usize = arg("--cycles", 12);
    let observes: usize = arg("--observes", 100);
    let inferences: usize = arg("--inferences", 60);
    let data = pegoles_computer::pegoles_data_dir();
    let computers = data.join("computers");
    let me = std::process::id() as i32;
    let foreign_vms = pids("com.apple.Virtualization.VirtualMachine", false);
    let mut failures: Vec<String> = Vec::new();

    let bus = EventBus::new();
    let shared = Shared(Arc::new(Mutex::new(Core {
        registry: ComputerRegistry::with_backend_kind(
            bus.clone(),
            BackendKind::MacOSVirtualization,
        ),
        tasks: TaskManager::new(bus.clone()),
    })));
    {
        let pump = shared.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            if let Ok(mut g) = pump.0.try_lock() {
                g.registry.pump();
            }
        });
    }
    shared.with_core(|r, _| {
        r.install_display_backend(Box::new(pegoles_computer::TestDisplay::new()))
    });
    let computer = CoreComputer::new(shared.clone(), bus);
    let started = Instant::now();
    let mut samples = Vec::new();
    let mut last_png: Option<Vec<u8>> = None;
    let mut observe_ms: Vec<u128> = Vec::new();
    let mut transient = 0usize;

    for cycle in 1..=cycles {
        let t = Instant::now();
        if let Err(e) = computer.prepare(&CancellationToken::new()) {
            failures.push(format!("cycle {cycle}: prepare failed: {e}"));
            break;
        }
        let boot_ms = t.elapsed().as_millis();
        let task = shared.with_core(|_, tm| tm.submit_task("release soak").unwrap().id);
        computer.set_status(task, TaskStatus::Running).ok();
        for i in 1..=observes {
            let o = Instant::now();
            let shot = computer
                .observe(task, &CancellationToken::new())
                .or_else(|e| {
                    transient += 1;
                    eprintln!("cycle {cycle} observe {i}: {e} (retrying once)");
                    computer.observe(task, &CancellationToken::new())
                });
            match shot {
                Ok(s) => {
                    observe_ms.push(o.elapsed().as_millis());
                    if i == observes {
                        last_png = Some(s.png);
                    }
                }
                Err(e) => {
                    failures.push(format!("cycle {cycle}: observe {i} failed twice: {e}"));
                    break;
                }
            }
            if i % 25 == 0 {
                let typed = computer.act(
                    task,
                    ComputerAction::TypeText {
                        text: format!("echo soak {cycle} {i}\n"),
                        sensitive: false,
                    },
                    &CancellationToken::new(),
                );
                if !typed.success {
                    failures.push(format!("cycle {cycle}: typing failed: {:?}", typed.error));
                }
            }
        }
        // Reset returns to the sealed image; then tear everything down.
        let reset = shared.with_core(|r, _| r.reset().map(|_| ()));
        if let Err(e) = reset {
            failures.push(format!("cycle {cycle}: reset failed: {e}"));
        }
        computer.set_status(task, TaskStatus::Completed).ok();
        let _ = shared.with_core(|r, _| r.destroy());
        std::thread::sleep(Duration::from_millis(1500));
        let vms: BTreeSet<i32> = pids("com.apple.Virtualization.VirtualMachine", false)
            .difference(&foreign_vms)
            .copied()
            .collect();
        let helpers = pids("pegoles-vm-host", true);
        let left = computers_in(&computers);
        let sample = serde_json::json!({
            "cycle": cycle,
            "boot_ms": boot_ms,
            "cycle_ms": t.elapsed().as_millis(),
            "self_footprint_mb": footprint_mb(me),
            "self_fds": own_fds(),
            "vm_processes_after_destroy": vms.len(),
            "helpers_after_destroy": helpers.len(),
            "computers_after_destroy": left,
            "computers_dir_kb": dir_kb(&computers),
        });
        println!("{sample}");
        if !vms.is_empty() || !helpers.is_empty() || left != 0 {
            failures.push(format!(
                "cycle {cycle}: left behind vms={vms:?} helpers={helpers:?} computers={left}"
            ));
        }
        samples.push(sample);
    }

    // Host growth across cycles (skip the first, which warms caches).
    let get = |s: &serde_json::Value, k: &str| s[k].as_u64().unwrap_or(0);
    if samples.len() >= 3 {
        let (a, b) = (&samples[1], &samples[samples.len() - 1]);
        let mem_growth = get(b, "self_footprint_mb") as i64 - get(a, "self_footprint_mb") as i64;
        let fd_growth = get(b, "self_fds") as i64 - get(a, "self_fds") as i64;
        println!(
            "host growth cycle 2 -> {}: footprint {mem_growth} MB, fds {fd_growth}",
            samples.len()
        );
        if mem_growth > 100 {
            failures.push(format!("host memory grew {mem_growth} MB across cycles"));
        }
        if fd_growth > 4 {
            failures.push(format!(
                "host descriptors grew by {fd_growth} across cycles"
            ));
        }
    }
    observe_ms.sort_unstable();
    let pct = |p: usize| {
        observe_ms
            .get(observe_ms.len().saturating_sub(1) * p / 100)
            .copied()
    };
    println!(
        "observes: {} (p50 {:?} ms, p95 {:?} ms, transient retries {transient}) in {:.0} s",
        observe_ms.len(),
        pct(50),
        pct(95),
        started.elapsed().as_secs_f64()
    );

    // Inference soak with forced worker restarts.
    if inferences > 0 {
        let catalog = Catalog::builtin();
        let spec = catalog
            .get(&catalog.default_model)
            .cloned()
            .expect("default model in catalog");
        let store = ModelStore::new(models_dir(&data));
        match (store.verify(&spec), last_png) {
            (Ok(verified), Some(png)) => {
                let cfg = MlxWorkerConfig::new(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../target/pegoles-runtime/python/bin/python3.12"),
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../workers/mlx/pegoles_mlx_worker.py"),
                    models_dir(&data),
                );
                let mut w = MlxWorkerBackend::new(cfg);
                let png = Arc::new(png);
                let req = GenerateRequest {
                    messages: vec![
                        ChatMessage::text(Role::User, "Task: Close the terminal window."),
                        ChatMessage {
                            role: Role::User,
                            parts: vec![Part::Image],
                        },
                    ],
                    images: vec![ImageInput {
                        png: png.clone(),
                        crop: None,
                        resize: Some((1024, 640)),
                    }],
                    max_tokens: 64,
                    temperature: 0.0,
                    timeout: Duration::from_secs(120),
                };
                let mut worker_mb = Vec::new();
                let mut restarts = 0;
                for i in 1..=inferences {
                    if let Err(e) = w.ensure_loaded(&verified, &|| false) {
                        failures.push(format!("inference {i}: load failed: {e}"));
                        break;
                    }
                    if let Err(e) = w.generate(&req, &|| false) {
                        // Like the local planner: a crashed worker is
                        // reported once, then respawned and reloaded.
                        let recovered =
                            matches!(e, pegoles_inference::InferenceError::WorkerCrashed(_))
                                && w.ensure_loaded(&verified, &|| false).is_ok()
                                && w.generate(&req, &|| false).is_ok();
                        if !recovered {
                            failures.push(format!("inference {i}: generate failed: {e}"));
                            break;
                        }
                    }
                    if let Some(pid) = w.pid() {
                        worker_mb.push(footprint_mb(pid));
                        if i % 20 == 0 && i < inferences {
                            let _ = std::process::Command::new("/bin/kill")
                                .args(["-9", &pid.to_string()])
                                .status();
                            restarts += 1;
                        }
                    }
                }
                let tmp_kb = std::env::var("TMPDIR")
                    .ok()
                    .map(|t| dir_kb(&PathBuf::from(t).join("pegoles-mlx")))
                    .unwrap_or(0);
                w.shutdown();
                std::thread::sleep(Duration::from_millis(1000));
                let workers = pids("pegoles_mlx_worker.py", false);
                println!(
                    "inference: {} requests, {restarts} forced restarts (worker restarts seen {}), worker footprint MB min {:?} max {:?}, temp dir {tmp_kb} KB, workers after shutdown {}",
                    worker_mb.len(),
                    w.restarts,
                    worker_mb.iter().min(),
                    worker_mb.iter().max(),
                    workers.len()
                );
                if !workers.is_empty() {
                    failures.push(format!("worker processes left: {workers:?}"));
                }
                if tmp_kb > 50 * 1024 {
                    failures.push(format!("worker temp dir grew to {tmp_kb} KB"));
                }
            }
            (Err(e), _) => failures.push(format!("default model not installed/verified: {e}")),
            (_, None) => failures.push("no screenshot for inference".into()),
        }
    }

    if failures.is_empty() {
        println!("RELEASE SOAK OK");
    } else {
        for f in &failures {
            eprintln!("FAIL: {f}");
        }
        eprintln!("RELEASE SOAK FAIL");
        std::process::exit(2);
    }
}

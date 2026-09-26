//! Capture soak on real hardware: N screen observations through the
//! product path must not grow guest memory. Regression test for the
//! memfd leak that let ~260 captures OOM-kill the guest compositor.
//!
//! ```sh
//! cargo build --release -p pegoles-agent --example capture_soak
//! cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
//! ./target/release/examples/capture_soak 400
//! ```

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pegoles_agent::{AgentComputer, CoreAccess, CoreComputer};
use pegoles_computer::platform::BackendKind;
use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, TaskManager};

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

fn available_mb(shared: &Shared) -> Option<u64> {
    shared.with_core(|r, _| {
        r.guest_info_request(Duration::from_secs(5))
            .ok()
            .and_then(|i| i.mem_available_mb)
    })
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
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
    computer
        .prepare(&CancellationToken::new())
        .expect("computer ready");
    let task = shared.with_core(|_, t| t.submit_task("capture soak").unwrap().id);
    shared.with_core(|_, t| t.start_task(&task)).unwrap();
    let before = available_mb(&shared);
    let t = Instant::now();
    let mut samples = Vec::new();
    let mut ok = true;
    for i in 1..=n {
        if let Err(e) = computer.observe(task, &CancellationToken::new()) {
            eprintln!("observe {i} failed: {e}");
            ok = false;
            break;
        }
        if i % 50 == 0 {
            let a = available_mb(&shared);
            println!("{i:>4} captures: guest MemAvailable {a:?} MiB");
            samples.push(a);
        }
    }
    let after = available_mb(&shared);
    let secs = t.elapsed().as_secs_f64();
    shared.with_core(|r, _| r.destroy()).ok();
    let drop_mb = match (before, after) {
        (Some(b), Some(a)) => b as i64 - a as i64,
        _ => i64::MAX,
    };
    println!(
        "{n} captures in {secs:.1} s ({:.0} ms each); guest MemAvailable {before:?} -> {after:?} MiB (drop {drop_mb} MiB)",
        secs * 1000.0 / n as f64
    );
    // A leak of one 1440x900 frame per capture would be ~5 MiB each.
    if ok && drop_mb < 150 {
        println!("CAPTURE SOAK OK");
    } else {
        eprintln!("CAPTURE SOAK FAIL");
        std::process::exit(2);
    }
}

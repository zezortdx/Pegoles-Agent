//! Real local-first acceptance test through the DESKTOP APP's own code:
//! `agent::start_run` picks the planner from the user's settings (Pegoles
//! Local by default), `local::LocalModels` verifies the installed model
//! and starts the host MLX worker, the product runner drives the real VM
//! from the sealed image through Core and Pegoles Policy. No scripted
//! planner, no cloud model, no mock VM, no pre-recorded screenshots.
//!
//! ```sh
//! cargo build --release -p pegoles-desktop --example local_e2e
//! cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
//! ./target/release/examples/local_e2e
//! ```
//! It also counts internet sockets held by Pegoles' processes (0 expected).
//! Wrapping it in `sandbox-exec` does not work: the model worker applies
//! its own sandbox, which macOS refuses inside another one; to check the
//! offline guarantee end to end, turn the network off instead.
//!
//! Scenario: (1) no cloud key, provider = local, model installed;
//! (2) a natural-language task runs to completion and is verified inside
//! the guest; (3) a second task is stopped by the user mid-run;
//! (4) teardown: VM destroyed, worker stopped; memory sampled throughout;
//! no internet sockets opened by the Pegoles processes.

fn main() {
    #[cfg(target_os = "macos")]
    mac::main();
    #[cfg(not(target_os = "macos"))]
    eprintln!("local_e2e drives the macOS VM and MLX worker; it runs on macOS only");
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use pegoles_agent::{AgentComputer, CoreComputer};
    use pegoles_core::CancellationToken;
    use pegoles_desktop::agent::{self, AgentSupervisor, AppCore, Provider};
    use pegoles_desktop::commands::lock_state;
    use pegoles_desktop::local::LocalModels;
    use pegoles_desktop::state::AppState;
    use pegoles_inference::hardware;
    use pegoles_protocol::{AgentEvent, ComputerAction, TaskId, TaskStatus};

    fn fail(msg: &str) -> ! {
        eprintln!("LOCAL E2E FAIL: {msg}");
        std::process::exit(2);
    }

    fn vm_pids() -> Vec<i32> {
        std::process::Command::new("/usr/bin/pgrep")
            .args(["-f", "com.apple.Virtualization.VirtualMachine"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| l.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn worker_pid() -> Option<i32> {
        let out = std::process::Command::new("/usr/bin/pgrep")
            .args(["-f", "pegoles_mlx_worker.py"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .next_back()
    }

    /// Internet sockets held by these processes (lsof), excluding none.
    fn inet_sockets(pids: &[i32]) -> Vec<String> {
        let list: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
        if list.is_empty() {
            return vec![];
        }
        std::process::Command::new("/usr/sbin/lsof")
            .args(["-nP", "-a", "-i", "-p", &list.join(",")])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .skip(1)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn gb(v: Option<u64>) -> String {
        v.map_or("-".into(), |b| format!("{:.2} GB", b as f64 / 1e9))
    }

    fn wait_end(
        shared: &Arc<Mutex<AppState>>,
        task: TaskId,
        limit: Duration,
    ) -> (TaskStatus, u128) {
        let t = Instant::now();
        loop {
            let status = lock_state(shared).tasks.get(&task).map(|t| t.status);
            match status {
                Ok(s @ (TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled)) => {
                    return (s, t.elapsed().as_millis())
                }
                Ok(_) if t.elapsed() < limit => std::thread::sleep(Duration::from_millis(200)),
                other => fail(&format!("task did not end in time: {other:?}")),
            }
        }
    }

    pub fn main() {
        let t0 = Instant::now();
        let hw = hardware::detect();
        let mem0 = hardware::system_memory();
        println!(
            "host: {:?}, {:.0} GiB, system used {} (pressure {:?})",
            hw.chip,
            hw.total_memory_gib(),
            gb(mem0.map(|m| m.used_bytes)),
            mem0.and_then(|m| m.pressure_level)
        );

        // (1) No cloud, local chosen, model installed.
        if agent::load_api_key().is_some() {
            fail("a cloud API key is configured; this test proves the keyless path");
        }
        let settings = agent::load_settings();
        if settings.provider != Provider::Local {
            fail("the chosen provider is not Pegoles Local");
        }
        let local = LocalModels::default();
        if !local.ready(&settings.local_model) {
            fail(&format!(
                "{} is not installed / runtime missing",
                settings.local_model
            ));
        }
        println!(
            "provider: local ({}), no cloud key configured",
            settings.local_model
        );

        // App state as the app builds it (real VM backend), plus a test display
        // adapter the app does not install yet (native_display.rs is a stub):
        // with it Core configures a DesktopLarge display (1440x900, the helper's
        // fixed scanout); the app creates its computer with `display: None`.
        let shared: Arc<Mutex<AppState>> = Arc::new(Mutex::new(AppState::new()));
        lock_state(&shared)
            .registry
            .install_display_backend(Box::new(pegoles_computer::TestDisplay::new()));
        let supervisor = AgentSupervisor::default();
        {
            let pump = shared.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                if let Ok(mut g) = pump.try_lock() {
                    g.registry.pump();
                }
            });
        }
        // Live narration, as the UI would show it.
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        {
            let mut rx = lock_state(&shared).subscribe();
            let log = log.clone();
            std::thread::spawn(move || {
                let t = Instant::now();
                while let Ok(e) = rx.blocking_recv() {
                    let line = match e {
                        AgentEvent::AgentMessage { kind, text, .. } => {
                            format!("{kind:?}: {}", text.chars().take(160).collect::<String>())
                        }
                        AgentEvent::ActionCompleted { request, result } => format!(
                            "action {} -> {:?}",
                            request.action.describe(true),
                            result.outcome
                        ),
                        AgentEvent::ActionDenied { request, reason } => {
                            format!("DENIED {}: {reason}", request.action.verb())
                        }
                        AgentEvent::FrameObserved { .. } => "screen observed".into(),
                        AgentEvent::TaskStatusChanged { to, .. } => format!("task -> {to:?}"),
                        _ => continue,
                    };
                    println!("  [{:>6} ms] {line}", t.elapsed().as_millis());
                    log.lock().unwrap().push(line);
                }
            });
        }
        let bus = lock_state(&shared).bus.clone();
        let other_vms = vm_pids();

        // (2) A natural-language task, end to end.
        let objective = "In the terminal, create a file named hello.txt in the home directory \
                         containing the words pegoles local, then show its contents with cat.";
        let task = lock_state(&shared).tasks.submit_task(objective).unwrap().id;
        let started = Instant::now();
        agent::start_run(
            shared.clone(),
            bus.clone(),
            supervisor.clone(),
            local.clone(),
            task,
        )
        .unwrap_or_else(|e| fail(&format!("start_run: {e}")));
        let mut samples = Vec::new();
        let mut sockets = Vec::new();
        let (status, _) = {
            let t = Instant::now();
            loop {
                let s = lock_state(&shared)
                    .tasks
                    .get(&task)
                    .map(|t| t.status)
                    .unwrap();
                let vm = vm_pids().into_iter().find(|p| !other_vms.contains(p));
                let w = worker_pid();
                samples.push((
                    w.and_then(hardware::process_memory)
                        .map(|m| m.phys_footprint_bytes),
                    vm.and_then(hardware::process_memory)
                        .map(|m| m.phys_footprint_bytes),
                    hardware::process_memory(std::process::id() as i32)
                        .map(|m| m.phys_footprint_bytes),
                    hardware::system_memory().map(|m| m.used_bytes),
                ));
                let mut pids = vec![std::process::id() as i32];
                pids.extend(w);
                sockets.extend(inet_sockets(&pids));
                if matches!(
                    s,
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                ) {
                    break (s, t.elapsed());
                }
                if t.elapsed() > Duration::from_secs(420) {
                    fail("task A did not finish in 7 minutes");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        };
        let task_a_ms = started.elapsed().as_millis();
        println!("task A: {status:?} in {task_a_ms} ms");

        // Verify inside the guest with a scripted check through the same
        // policy path (not part of the model's run).
        let computer = CoreComputer::new(AppCore(shared.clone()), bus.clone());
        let probe = lock_state(&shared).tasks.submit_task("verify").unwrap().id;
        lock_state(&shared).tasks.start_task(&probe).unwrap();
        computer
            .prepare(&CancellationToken::new())
            .unwrap_or_else(|e| fail(&format!("prepare for verify: {e}")));
        let run = |a: ComputerAction| {
            let r = computer.act(probe, a, &CancellationToken::new());
            if !r.success {
                fail(&format!("verify action failed: {}", r.message));
            }
        };
        run(ComputerAction::KeyChord {
            keys: vec!["Control".into(), "c".into()],
        });
        run(ComputerAction::TypeText {
            text: "clear; if grep -qi 'pegoles local' ~/hello.txt; then c=46; else c=196; fi; \
                   for i in $(seq 20); do printf \"\\033[48;5;${c}m%150s\\033[0m\\n\" ''; done\n"
                .into(),
            sensitive: false,
        });
        run(ComputerAction::Wait { duration_ms: 1000 });
        let shot = computer.observe(probe, &CancellationToken::new()).unwrap();
        let px = {
            let decoder = png::Decoder::new(std::io::Cursor::new(shot.png.clone()));
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut buf).unwrap();
            buf.truncate(info.buffer_size());
            buf
        };
        let count = |c: [u8; 3]| px.chunks_exact(3).filter(|p| *p == c).count();
        let (green, red) = (count([0, 255, 0]), count([255, 0, 0]));
        let verified = green > 20_000 && red < 1_000;
        println!("task A verified in guest: {verified} (green {green}, red {red})");
        lock_state(&shared).tasks.finish_task(&probe).ok();
        {
            let mut g = lock_state(&shared);
            let mut out = Vec::new();
            g.registry.end_agent_session(&mut out);
        }

        // (3) The user stops a running task.
        let long = "Type the numbers from 1 to 50 in the terminal, one command at a time, \
                    pressing Enter after each.";
        let task_b = lock_state(&shared).tasks.submit_task(long).unwrap().id;
        let before = log.lock().unwrap().len();
        agent::start_run(
            shared.clone(),
            bus.clone(),
            supervisor.clone(),
            local.clone(),
            task_b,
        )
        .unwrap_or_else(|e| fail(&format!("start_run B: {e}")));
        let t = Instant::now();
        while !log.lock().unwrap()[before..]
            .iter()
            .any(|l| l.starts_with("action "))
        {
            if t.elapsed() > Duration::from_secs(120) {
                fail("task B never acted");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let stop = Instant::now();
        if !supervisor.cancel(Some(task_b)) {
            fail("Stop did not reach the run");
        }
        let (b_status, _) = wait_end(&shared, task_b, Duration::from_secs(60));
        let stop_ms = stop.elapsed().as_millis();
        println!("task B: {b_status:?}, stopped {stop_ms} ms after Stop");
        let t = Instant::now();
        while supervisor.active().is_some() && t.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(50));
        }

        // (3b) The inference worker dies mid-task (kill -9): the planner
        // restarts it, reloads the verified model, and the task goes on.
        let task_c = lock_state(&shared)
            .tasks
            .submit_task("In the terminal, type echo recovered and press Enter.")
            .unwrap()
            .id;
        let before = log.lock().unwrap().len();
        agent::start_run(
            shared.clone(),
            bus.clone(),
            supervisor.clone(),
            local.clone(),
            task_c,
        )
        .unwrap_or_else(|e| fail(&format!("start_run C: {e}")));
        let t = Instant::now();
        while !log.lock().unwrap()[before..]
            .iter()
            .any(|l| l.starts_with("action "))
        {
            if t.elapsed() > Duration::from_secs(120) {
                fail("task C never acted");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let killed = worker_pid();
        if let Some(pid) = killed {
            // SAFETY: plain kill(2) on our own child worker.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let (c_status, c_ms) = wait_end(&shared, task_c, Duration::from_secs(300));
        let restarted = worker_pid().is_some_and(|p| Some(p) != killed);
        println!(
            "task C: worker {killed:?} killed mid-task -> {c_status:?} after {c_ms} ms; new worker running {restarted}"
        );

        // (4) Teardown.
        let worker = worker_pid();
        let vm = vm_pids().into_iter().find(|p| !other_vms.contains(p));
        let td = Instant::now();
        lock_state(&shared).registry.destroy().ok();
        local.shutdown();
        std::thread::sleep(Duration::from_millis(800));
        let vm_gone = vm.is_none_or(|p| hardware::process_memory(p).is_none());
        let worker_gone = worker.is_none_or(|p| hardware::process_memory(p).is_none());
        println!(
            "teardown {} ms: vm process gone {vm_gone}, worker gone {worker_gone}; system used after {}",
            td.elapsed().as_millis(),
            gb(hardware::system_memory().map(|m| m.used_bytes))
        );

        let peak = |i: usize| {
            samples
                .iter()
                .filter_map(|s| match i {
                    0 => s.0,
                    1 => s.1,
                    2 => s.2,
                    _ => s.3,
                })
                .max()
        };
        println!(
            "memory peak: worker {}, vm {}, app runtime (this process) {}, system used {}",
            gb(peak(0)),
            gb(peak(1)),
            gb(peak(2)),
            gb(peak(3))
        );
        sockets.sort();
        sockets.dedup();
        println!(
            "internet sockets held by Pegoles processes during task A: {}",
            sockets.len()
        );
        for s in &sockets {
            println!("  {s}");
        }
        let ok = status == TaskStatus::Completed
            && verified
            && killed.is_some()
            && c_status != TaskStatus::Cancelled
            && restarted
            && b_status == TaskStatus::Cancelled
            && vm_gone
            && worker_gone
            && sockets.is_empty();
        println!("total {} s", t0.elapsed().as_secs());
        if ok {
            println!("LOCAL E2E DONE");
        } else {
            fail("see above");
        }
    }
}

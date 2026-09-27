//! Real end-to-end proof on Apple Silicon (and, with the same code, on
//! Windows through the Host Compute System): the product orchestrator
//! (`run_task` + `CoreComputer`, the same code the desktop app runs)
//! drives a real Virtualization.framework VM booted from the sealed
//! Pegoles image. No mocks: every action crosses Core's executor,
//! Pegoles Policy, the Swift helper, vsock, and the guest runtime.
//!
//! ```sh
//! (cd native/macos/pegoles-vm-host && swift build -c release)
//! codesign --entitlements apps/desktop/src-tauri/entitlements/vm-host.plist -f -s - \
//!   native/macos/pegoles-vm-host/.build/release/pegoles-vm-host
//! cargo run -p pegoles-agent --example agent_e2e
//! ```
//!
//! Scenario:
//! 1. boot → guest ready → agent session;
//! 2. task A: click the terminal, type a command that paints a red block,
//!    press Enter, verify red pixels in a fresh frame (proves pointer,
//!    typing incl. dead-key characters, Enter, capture, channel order);
//! 3. task B: a long wait cancelled from another thread (cancel latency);
//! 4. policy: a forged out-of-screen action is denied, nothing reaches
//!    the guest;
//! 5. recovery: kill the guest runtime from inside the guest; the host
//!    sees the disconnect and the runtime reconnects;
//! 6. teardown, then a second session boots and observes successfully.
//!
//! On Windows (a debug build: the x64 image is sealed locally with
//! `seal_x64_image`, and release builds boot only pinned images), with
//! Pegoles' broker service installed and
//! `PEGOLES_VM_HOST_WINDOWS` pointing at the installed helper; CI's
//! `windows-guest-boot` job runs it that way.
//!
//! `PEGOLES_E2E_OUT=<dir>` saves the verification frames as PNG.
//! Exit 0 only when every assertion holds.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pegoles_agent::{
    run_task, AgentComputer, CoreAccess, CoreComputer, PlannedCall, RunEnd, RunLimits, Screenshot,
    ScriptedPlanner, Step,
};
use pegoles_computer::platform::BackendKind;
use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, TaskManager};
use pegoles_protocol::{
    ActionOutcome, ComputerAction, ControlOwner, GuestRuntimeState, PointerButton, TaskStatus,
};

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

fn fail(msg: &str, shared: &Shared) -> ! {
    eprintln!("AGENT E2E FAIL: {msg}");
    shared.with_core(|r, _| {
        match r.read_boot_log(40) {
            Ok(log) => {
                eprintln!("serial log: {} lines shown", log.tail.len());
                for line in log.tail {
                    eprintln!("serial: {line}");
                }
            }
            Err(e) => eprintln!("serial log unavailable: {e}"),
        }
        let _ = r.destroy();
    });
    std::process::exit(2);
}

fn call(id: &str, label: &str, steps: Vec<Step>) -> PlannedCall {
    PlannedCall {
        call_id: id.to_string(),
        label: label.to_string(),
        steps: Ok(steps),
    }
}

fn act(a: ComputerAction) -> Step {
    Step::Act(a)
}

/// Decode a PNG and count strongly red and strongly blue pixels.
fn color_counts(shot: &Screenshot) -> (usize, usize) {
    let decoder = png::Decoder::new(std::io::Cursor::new(shot.png.clone()));
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let step = info.color_type.samples();
    let (mut red, mut blue) = (0, 0);
    for px in buf[..info.buffer_size()].chunks_exact(step) {
        let (r, g, b) = (px[0], px[1], px[2]);
        let (r, g, b) = (r as i32, g as i32, b as i32);
        // Red-dominant (foot's ANSI red is ~#f62b5a). A red/blue channel
        // swap would turn every one of these into a blue-dominant pixel.
        if r > 150 && r > g + 100 && r > b + 80 {
            red += 1;
        }
        if b > 150 && b > g + 100 && b > r + 80 {
            blue += 1;
        }
    }
    (red, blue)
}

fn pixels(shot: &Screenshot) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::Cursor::new(shot.png.clone()));
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    buf.truncate(info.buffer_size());
    buf
}

/// Pixels that differ between two frames of the same size.
fn diff(a: &Screenshot, b: &Screenshot) -> usize {
    let (pa, pb) = (pixels(a), pixels(b));
    pa.chunks_exact(3)
        .zip(pb.chunks_exact(3))
        .filter(|(x, y)| x != y)
        .count()
}

fn save(shot: &Screenshot, name: &str) {
    if let Ok(dir) = std::env::var("PEGOLES_E2E_OUT") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, &shot.png).expect("save frame");
        println!("saved {}", path.display());
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn boot(shared: &Shared) -> (u64, u64) {
    let t0 = Instant::now();
    shared.with_core(|r, _| {
        r.install_display_backend(Box::new(pegoles_computer::TestDisplay::new()));
    });
    let computer = CoreComputer::new(shared.clone(), EventBus::new());
    if let Err(e) = computer.prepare(&CancellationToken::new()) {
        fail(&format!("prepare: {e}"), shared);
    }
    let ready_ms = shared.with_core(|r, _| {
        let mut out = Vec::new();
        r.end_agent_session(&mut out);
        r.guest_ready_ms().unwrap_or(0)
    });
    (t0.elapsed().as_millis() as u64, ready_ms)
}

fn main() {
    println!("image: {}", pegoles_computer::active_image_id());
    let bus = EventBus::new();
    let shared = Shared(Arc::new(Mutex::new(Core {
        // The host's own backend: Virtualization.framework on a Mac, the
        // Host Compute System (through the broker service) on Windows.
        registry: ComputerRegistry::with_backend_kind(
            bus.clone(),
            if cfg!(windows) {
                BackendKind::WindowsHcs
            } else {
                BackendKind::MacOSVirtualization
            },
        ),
        tasks: TaskManager::new(bus.clone()),
    })));
    // Guest and computer lifecycle events, as they happen (diagnostics:
    // why a boot or a handshake failed).
    {
        let mut events = bus.subscribe();
        std::thread::spawn(move || {
            while let Ok(event) = events.blocking_recv() {
                let text = format!("{event:?}");
                if text.starts_with("Guest")
                    || text.starts_with("Computer")
                    || text.starts_with("InputCapability")
                {
                    eprintln!("event: {}", text.chars().take(300).collect::<String>());
                }
            }
        });
    }
    // Core maintenance independent of any caller (the desktop app runs
    // the same loop): guest heartbeats keep flowing during waits.
    {
        let pump = shared.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            if let Ok(mut g) = pump.0.try_lock() {
                g.registry.pump();
            }
        });
    }
    let computer = CoreComputer::new(shared.clone(), bus.clone());

    // --- 1. boot ---------------------------------------------------------
    let (prepare_ms, guest_ready_ms) = boot(&shared);
    println!("boot: prepare {prepare_ms} ms (guest ready at {guest_ready_ms} ms after VM start)");

    // Observe latency (policy-checked ObserveScreen + PNG encode).
    let probe_task = shared.with_core(|_, t| t.submit_task("latency probe").unwrap().id);
    let mut observe_ms = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let shot = computer
            .observe(probe_task, &CancellationToken::new())
            .unwrap_or_else(|e| fail(&format!("observe: {e}"), &shared));
        observe_ms.push(t.elapsed().as_millis() as u64);
        assert_eq!((shot.width, shot.height), (1440, 900));
    }
    observe_ms.sort_unstable();

    // --- 2. task A: paint a red block, verify pixels ---------------------
    let task_a = shared.with_core(|_, t| {
        t.submit_task("Paint a red block in the terminal and confirm it")
            .unwrap()
            .id
    });
    let command = "clear; for i in $(seq 1 12); do printf '\\033[41m%150s\\033[0m\\n' ''; done";
    let turns = vec![
        vec![call(
            "focus",
            "left_click",
            vec![act(ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            })],
        )],
        vec![
            call(
                "type",
                "type",
                vec![act(ComputerAction::TypeText {
                    text: command.to_string(),
                    sensitive: false,
                })],
            ),
            call(
                "enter",
                "key",
                vec![act(ComputerAction::KeyPress {
                    key: "Enter".into(),
                })],
            ),
            call(
                "settle",
                "wait",
                vec![act(ComputerAction::Wait { duration_ms: 1200 })],
            ),
        ],
    ];
    let verdict = Arc::new(Mutex::new(None));
    let verdict2 = verdict.clone();
    let mut planner = ScriptedPlanner::new(turns, move |shot| {
        save(shot, "e2e-red-block.png");
        let (red, blue) = color_counts(shot);
        *verdict2.lock().unwrap() = Some((red, blue));
        if red > 50_000 && blue < 1_000 {
            Ok(format!("Red block painted ({red} red pixels)."))
        } else {
            Err(format!("expected a red block: red={red} blue={blue}"))
        }
    });
    let t = Instant::now();
    let report = run_task(
        task_a,
        "Paint a red block in the terminal and confirm it",
        &mut planner,
        &computer,
        &RunLimits::default(),
        &CancellationToken::new(),
    );
    let task_a_ms = t.elapsed().as_millis();
    println!(
        "task A: {:?} in {task_a_ms} ms, {} actions: {}",
        report.end, report.actions, report.summary
    );
    if report.end != RunEnd::Completed {
        fail(
            &format!("task A did not complete: {}", report.summary),
            &shared,
        );
    }
    let status = shared.with_core(|r, t| {
        (
            t.get(&task_a).unwrap().status,
            r.control_owner(),
            r.input_status().pressed_clean,
        )
    });
    assert_eq!(status, (TaskStatus::Completed, ControlOwner::None, true));

    // --- 2b. task D: scroll, double-click, drag (frame diffs) ------------
    let task_d = shared.with_core(|_, t| t.submit_task("scroll and select").unwrap().id);
    let shot = |id: &str| call(id, "screenshot", vec![Step::Observe]);
    let turns = vec![
        vec![
            call(
                "fill",
                "type",
                vec![act(ComputerAction::TypeText {
                    text: "clear; seq 1 400\n".into(),
                    sensitive: false,
                })],
            ),
            call(
                "settle",
                "wait",
                vec![act(ComputerAction::Wait { duration_ms: 800 })],
            ),
            shot("a"),
        ],
        vec![
            call(
                "scroll",
                "scroll",
                vec![act(ComputerAction::Scroll {
                    x: 0.5,
                    y: 0.5,
                    delta_x: 0.0,
                    delta_y: -15.0,
                })],
            ),
            call(
                "settle",
                "wait",
                vec![act(ComputerAction::Wait { duration_ms: 400 })],
            ),
            shot("b"),
        ],
        vec![
            call(
                "select-word",
                "double_click",
                vec![act(ComputerAction::DoubleClick {
                    x: 0.004,
                    y: 0.5,
                    button: PointerButton::Primary,
                })],
            ),
            call(
                "settle",
                "wait",
                vec![act(ComputerAction::Wait { duration_ms: 300 })],
            ),
            shot("c"),
        ],
        vec![
            call(
                "select-range",
                "left_click_drag",
                vec![act(ComputerAction::Drag {
                    from_x: 0.002,
                    from_y: 0.3,
                    to_x: 0.3,
                    to_y: 0.4,
                    button: PointerButton::Primary,
                    duration_ms: 500,
                })],
            ),
            call(
                "settle",
                "wait",
                vec![act(ComputerAction::Wait { duration_ms: 300 })],
            ),
            shot("d"),
        ],
    ];
    let frames = Arc::new(Mutex::new(Vec::new()));
    let frames2 = frames.clone();
    let mut planner =
        ScriptedPlanner::new(turns, |_| Ok("input checked".into())).observing(move |outcomes| {
            for o in outcomes {
                if let Ok(pegoles_agent::CallOutput::Image(img)) = &o.result {
                    frames2.lock().unwrap().push(img.clone());
                }
            }
        });
    let report = run_task(
        task_d,
        "scroll and select",
        &mut planner,
        &computer,
        &RunLimits::default(),
        &CancellationToken::new(),
    );
    let frames = frames.lock().unwrap().clone();
    if report.end != RunEnd::Completed || frames.len() < 4 {
        fail(
            &format!(
                "task D: {:?} {} frames: {}",
                report.end,
                frames.len(),
                report.summary
            ),
            &shared,
        );
    }
    let (scrolled, word, range) = (
        diff(&frames[0], &frames[1]),
        diff(&frames[1], &frames[2]),
        diff(&frames[2], &frames[3]),
    );
    save(&frames[1], "e2e-scrolled.png");
    save(&frames[3], "e2e-selection.png");
    println!("task D: scroll changed {scrolled} px, double-click {word} px, drag {range} px");
    if scrolled < 5_000 || word < 50 || range < 500 {
        fail(
            "scroll/double-click/drag did not visibly change the screen",
            &shared,
        );
    }

    // --- 3. task B: cancellation latency --------------------------------
    let task_b = shared.with_core(|_, t| t.submit_task("wait for a long time").unwrap().id);
    let long_waits = vec![vec![call(
        "long",
        "wait",
        vec![act(ComputerAction::Wait {
            duration_ms: 30_000,
        })],
    )]];
    let mut planner = ScriptedPlanner::new(long_waits, |_| Ok("unused".into()));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let cancelled_at = Arc::new(Mutex::new(None));
    let cancelled_at2 = cancelled_at.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        *cancelled_at2.lock().unwrap() = Some(Instant::now());
        trigger.cancel();
    });
    let report = run_task(
        task_b,
        "wait",
        &mut planner,
        &computer,
        &RunLimits::default(),
        &cancel,
    );
    let cancel_latency = cancelled_at
        .lock()
        .unwrap()
        .map(|at| at.elapsed().as_millis())
        .unwrap_or(u128::MAX);
    println!(
        "task B: {:?}, cancel honored in {cancel_latency} ms",
        report.end
    );
    if report.end != RunEnd::Cancelled || cancel_latency > 1_000 {
        fail("cancellation not honored within 1 s", &shared);
    }
    let control = shared.with_core(|r, _| r.control_owner());
    assert_eq!(control, ControlOwner::None, "control released after cancel");

    // --- 4. policy: forged out-of-screen input is denied -----------------
    let denied = shared.with_core(|r, _| {
        r.execute_action(
            probe_task,
            ComputerAction::Click {
                x: 4.0,
                y: -1.0,
                button: PointerButton::Primary,
            },
            false,
            &pegoles_policy::PolicyContext::default(),
            &CancellationToken::new(),
            true,
        )
        .0
    });
    println!(
        "policy: forged click → {:?} ({})",
        denied.outcome, denied.message
    );
    assert_eq!(denied.outcome, ActionOutcome::Blocked);

    // --- 5. recovery: kill the guest runtime from inside the guest ------
    let task_c = shared.with_core(|_, t| t.submit_task("restart the runtime").unwrap().id);
    let kill = vec![vec![
        call(
            "focus",
            "left_click",
            vec![act(ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            })],
        ),
        call(
            "kill",
            "type",
            vec![act(ComputerAction::TypeText {
                text: "pkill -f pegoles-guest-runtime; echo pkill=$?\n".into(),
                sensitive: false,
            })],
        ),
    ]];
    let mut planner = ScriptedPlanner::new(kill, |_| Ok("unused".into()));
    let mut guest_events = bus.subscribe();
    let t_kill = Instant::now();
    // The run itself will end (its final observation needs the runtime
    // it just killed); what matters is what the host does next.
    let _ = run_task(
        task_c,
        "restart",
        &mut planner,
        &computer,
        &RunLimits::default(),
        &CancellationToken::new(),
    );
    // The host must SEE the runtime die (disconnect event) and see it
    // come back (ready event) — exactly what the UI renders.
    let mut saw_disconnect = None;
    let mut recovered_ms = None;
    while t_kill.elapsed() < Duration::from_secs(40) && recovered_ms.is_none() {
        shared.with_core(|r, _| r.pump());
        loop {
            use pegoles_protocol::AgentEvent as E;
            use tokio::sync::broadcast::error::TryRecvError;
            match guest_events.try_recv() {
                Ok(E::GuestRuntimeDisconnected { reason, .. }) if saw_disconnect.is_none() => {
                    println!(
                        "recovery: disconnect ({reason}) at {} ms",
                        t_kill.elapsed().as_millis()
                    );
                    saw_disconnect = Some(reason);
                }
                Ok(E::GuestRuntimeReady { .. }) if saw_disconnect.is_some() => {
                    recovered_ms = Some(t_kill.elapsed().as_millis());
                    break;
                }
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let saw_disconnect = saw_disconnect.is_some();
    let state = shared.with_core(|r, _| (r.guest_state(), r.input_available()));
    if state != (GuestRuntimeState::Ready, true) {
        fail(&format!("not ready after recovery: {state:?}"), &shared);
    }
    println!("recovery: disconnect seen={saw_disconnect}, ready again after {recovered_ms:?} ms");
    if recovered_ms.is_none() {
        if let Ok(shot) = computer.observe(probe_task, &CancellationToken::new()) {
            save(&shot, "e2e-recovery-failed.png");
        }
        fail("guest runtime did not recover", &shared);
    }
    let after = computer
        .observe(probe_task, &CancellationToken::new())
        .unwrap_or_else(|e| fail(&format!("observe after recovery: {e}"), &shared));
    save(&after, "e2e-after-recovery.png");

    // --- 6. teardown + second session -----------------------------------
    let t = Instant::now();
    shared.with_core(|r, _| {
        r.stop().unwrap_or_else(|e| panic!("stop: {e}"));
        r.destroy().unwrap_or_else(|e| panic!("destroy: {e}"));
    });
    let teardown_ms = t.elapsed().as_millis();
    let (second_prepare_ms, _) = boot(&shared);
    let second = computer
        .observe(probe_task, &CancellationToken::new())
        .unwrap_or_else(|e| fail(&format!("second session observe: {e}"), &shared));
    assert_eq!((second.width, second.height), (1440, 900));
    shared.with_core(|r, _| {
        let _ = r.stop();
        let _ = r.destroy();
    });

    let (red, blue) = verdict.lock().unwrap().unwrap_or((0, 0));
    println!(
        "metrics: {}",
        serde_json::json!({
            "prepare_ms": prepare_ms,
            "guest_ready_ms": guest_ready_ms,
            "observe_ms_p50": percentile(&observe_ms, 0.5),
            "observe_ms_p95": percentile(&observe_ms, 0.95),
            "task_a_ms": task_a_ms,
            "red_pixels": red,
            "scroll_diff_px": scrolled,
            "double_click_diff_px": word,
            "drag_diff_px": range,
            "blue_pixels": blue,
            "cancel_latency_ms": cancel_latency,
            "runtime_recovery_ms": recovered_ms,
            "teardown_ms": teardown_ms,
            "second_session_prepare_ms": second_prepare_ms,
        })
    );
    println!("AGENT E2E DONE");
}

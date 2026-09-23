//! Phase 5.1 real-hardware Eyes & Hands E2E against Image v2.
//!
//! ```sh
//! PEGOLES_IMAGE_ID=pegoles-base-0.2 cargo run -p pegoles-core --example v2_e2e
//! ```
//!
//! Boots v2 with the fixture fullscreen, then drives every primitive
//! through the REAL executor (policy -> arbiter -> guest channel), saving
//! each observed guest frame as PNG under `$PEGOLES_E2E_DIR` (default
//! /tmp/pegoles-e2e) for pixel verification. Also exercises takeover
//! during a held button (release-all), pause mid-action (no replay),
//! heartbeat-loss reconnect, and records the Phase 5.1 performance
//! facts. Host pointer position is sampled before/after to prove the
//! host cursor is never moved. No model, no UI, no fakes.

use pegoles_computer::platform::BackendKind;
use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, ScriptReport, ScriptStep};
use pegoles_policy::PolicyContext;
use pegoles_protocol::{
    ComputerAction as A, GraphicalSessionState, GuestRuntimeState, PointerButton,
};
use std::time::{Duration, Instant};

// Fixture region centers (guest/fixture layout(): proportional regions,
// foot fullscreen so terminal cells map 1:1 onto normalized screen space).
const CLICK_BTN: (f64, f64) = (0.37, 0.33);
const TYPE_FIELD: (f64, f64) = (0.50, 0.51);
const DRAG_SRC: (f64, f64) = (0.16, 0.71);
const DRAG_DST: (f64, f64) = (0.84, 0.71);
const SCROLL_LIST: (f64, f64) = (0.62, 0.27);

fn host_pointer() -> String {
    let out = std::process::Command::new("osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            "ObjC.import('AppKit'); var p = $.NSEvent.mouseLocation; p.x + ',' + p.y",
        ])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(e) => format!("unavailable: {e}"),
    }
}

fn wait_guest(registry: &mut ComputerRegistry, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        let _ = registry.pump();
        if registry.guest_state() == GuestRuntimeState::Ready
            && registry.graphical_session().state == GraphicalSessionState::Ready
            && registry.input_available()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

struct Harness {
    registry: ComputerRegistry,
    out_dir: std::path::PathBuf,
    failures: Vec<String>,
}

impl Harness {
    fn run(
        &mut self,
        label: &str,
        steps: &[ScriptStep],
        cancel: &CancellationToken,
    ) -> ScriptReport {
        let (report, events) = self.registry.run_script(
            pegoles_protocol::TaskId::new(),
            steps,
            &PolicyContext::default(),
            cancel,
        );
        let kinds: Vec<String> = events
            .iter()
            .map(|e| {
                serde_json::to_value(e).expect("event serializes")["type"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string()
            })
            .collect();
        println!("[{label}] events: {}", kinds.join(","));
        for (i, r) in report.results.iter().enumerate() {
            println!(
                "[{label}] step {i} {:?} success={} {}ms frame={} msg={}",
                steps[i].label,
                r.success,
                r.duration_ms,
                r.resulting_frame_id.is_some(),
                r.message
            );
        }
        self.save_frame(label);
        report
    }

    fn expect_ok(&mut self, label: &str, steps: &[ScriptStep]) -> ScriptReport {
        let report = self.run(label, steps, &CancellationToken::new());
        if report.aborted_at.is_some() || report.steps_executed != report.steps_total {
            self.failures
                .push(format!("{label}: aborted at {:?}", report.aborted_at));
        }
        report
    }

    fn save_frame(&self, label: &str) {
        if let Some((meta, rgba)) = self.registry.last_frame_bytes() {
            match pegoles_computer::encode_png_rgba(meta.width_px, meta.height_px, &rgba) {
                Ok(png) => {
                    let path = self.out_dir.join(format!("{label}.png"));
                    let _ = std::fs::write(&path, png);
                    println!(
                        "[{label}] frame {}x{} capture_latency={}ms -> {}",
                        meta.width_px,
                        meta.height_px,
                        meta.capture_latency_ms,
                        path.display()
                    );
                }
                Err(e) => println!("[{label}] png encode failed: {e}"),
            }
        }
    }

    fn check_clean(&mut self, label: &str) {
        let st = self.registry.input_status();
        println!(
            "[{label}] agent_busy={} pressed_clean={} owner={:?}",
            st.agent_busy,
            st.pressed_clean,
            self.registry.control_owner()
        );
        if st.agent_busy || !st.pressed_clean {
            self.failures.push(format!("{label}: dirty input state"));
        }
    }
}

fn reconnect_check(h: &mut Harness) {
    // Reconnect: hold paused past the heartbeat budget so the host kicks
    // the guest session; after resume the runtime must reconnect with
    // capabilities restored.
    h.registry.pause().expect("pause for reconnect");
    std::thread::sleep(Duration::from_secs(35));
    for e in h.registry.pump() {
        println!(
            "reconnect pump (paused): {}",
            serde_json::to_value(&e).expect("event")["type"]
        );
    }
    println!(
        "reconnect: guest state while paused = {:?}",
        h.registry.guest_state()
    );
    h.registry.resume().expect("resume");
    let reconnect = Instant::now();
    let mut last = h.registry.guest_state();
    let mut ready = false;
    while reconnect.elapsed() < Duration::from_secs(90) {
        for e in h.registry.pump() {
            let v = serde_json::to_value(&e).expect("event");
            if v["type"].as_str().is_some_and(|t| t.contains("error")) {
                println!("reconnect pump: {v}");
            } else {
                println!("reconnect pump: {}", v["type"]);
            }
        }
        let now = h.registry.guest_state();
        if now != last {
            println!(
                "reconnect: {last:?} -> {now:?} at {}ms",
                reconnect.elapsed().as_millis()
            );
            last = now;
        }
        if now == GuestRuntimeState::Ready
            && h.registry.graphical_session().state == GraphicalSessionState::Ready
            && h.registry.input_available()
        {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if ready {
        println!(
            "reconnect: ready after {}ms, input={} frame={}",
            reconnect.elapsed().as_millis(),
            h.registry.input_available(),
            h.registry.input_status().frame_available
        );
        h.expect_ok(
            "15-after-reconnect",
            &[click("click", CLICK_BTN).observing()],
        );
    } else {
        h.failures
            .push(format!("reconnect: guest {last:?} after resume"));
    }
}

fn step(label: &str, action: A) -> ScriptStep {
    ScriptStep::new(label, action)
}

fn click(label: &str, (x, y): (f64, f64)) -> ScriptStep {
    step(
        label,
        A::Click {
            x,
            y,
            button: PointerButton::Primary,
        },
    )
}

fn key(name: &str) -> ScriptStep {
    step(
        name,
        A::KeyPress {
            key: name.to_string(),
        },
    )
}

fn main() {
    let image_id = std::env::var("PEGOLES_IMAGE_ID").unwrap_or_else(|_| "pegoles-base-0.2".into());
    let out_dir = std::path::PathBuf::from(
        std::env::var("PEGOLES_E2E_DIR").unwrap_or_else(|_| "/tmp/pegoles-e2e".into()),
    );
    std::fs::create_dir_all(&out_dir).expect("out dir");
    println!("image: {image_id}");
    let mut registry =
        ComputerRegistry::with_backend_kind(EventBus::new(), BackendKind::MacOSVirtualization);
    registry.install_display_backend(Box::new(pegoles_computer::TestDisplay::new()));
    let id = registry.create_default().expect("create");
    println!("computer: {id}");
    let boot = Instant::now();
    registry.start().expect("start");
    if !wait_guest(&mut registry, Duration::from_secs(180)) {
        eprintln!(
            "E2E FAIL: guest={:?} graphical={:?} input={} unavailable={:?}",
            registry.guest_state(),
            registry.graphical_session().state,
            registry.input_available(),
            registry.input_status().unavailable
        );
        std::process::exit(2);
    }
    println!(
        "perf: guest_ready_ms={:?} graphical_ready_ms={:?} display_ready_ms={:?} wall_to_all_ready={}ms",
        registry.guest_ready_ms(),
        registry.graphical_session().ready_in_ms,
        registry.display_ready_ms(),
        boot.elapsed().as_millis()
    );
    // Attach the (test) display view so control ownership can be taken,
    // exactly as the app does when the viewport mounts.
    let geometry = pegoles_computer::DisplayGeometry {
        rect: pegoles_computer::DisplayRect {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        },
        visible: true,
        animate_ms: 0,
    };
    let bounds = pegoles_core::DisplayBounds {
        width: 1440.0,
        height: 900.0,
        scale: 1.0,
    };
    println!(
        "display attach: {:?}",
        registry.set_display_geometry(geometry, bounds)
    );
    let _ = registry.pump_display();
    let mut h = Harness {
        registry,
        out_dir,
        failures: Vec::new(),
    };
    // Let the fixture paint its first frame.
    std::thread::sleep(Duration::from_secs(3));

    if std::env::var("PEGOLES_E2E_RECONNECT_ONLY").is_ok() {
        reconnect_check(&mut h);
        finish(h);
        return;
    }
    let host_before = host_pointer();
    h.expect_ok(
        "00-baseline",
        &[step("observe", A::ObserveScreen).observing()],
    );
    h.expect_ok(
        "01-move",
        &[step(
            "move",
            A::MovePointer {
                x: CLICK_BTN.0,
                y: CLICK_BTN.1,
            },
        )
        .observing()],
    );
    let host_after = host_pointer();
    println!("host pointer before={host_before} after={host_after}");
    if host_before != host_after {
        h.failures.push("host pointer moved".into());
    }
    h.expect_ok("02-click", &[click("click", CLICK_BTN).observing()]);
    h.expect_ok(
        "03-doubleclick",
        &[step(
            "double",
            A::DoubleClick {
                x: CLICK_BTN.0,
                y: CLICK_BTN.1,
                button: PointerButton::Primary,
            },
        )
        .observing()],
    );
    h.expect_ok(
        "04-type",
        &[
            click("focus field", TYPE_FIELD),
            step(
                "type",
                A::TypeText {
                    text: "Pegoles is alive".into(),
                    sensitive: false,
                },
            )
            .observing(),
        ],
    );
    h.expect_ok("04b-enter", &[key("Enter").observing()]);
    h.expect_ok(
        "05-accents",
        &[
            click("focus field", TYPE_FIELD),
            step(
                "type",
                A::TypeText {
                    text: "ação café perché è à".into(),
                    sensitive: false,
                },
            )
            .observing(),
        ],
    );
    h.expect_ok("05b-enter", &[key("Enter").observing()]);
    h.expect_ok(
        "06-keys",
        &[
            click("focus field", TYPE_FIELD),
            step(
                "type",
                A::TypeText {
                    text: "abc".into(),
                    sensitive: false,
                },
            ),
            key("ArrowLeft"),
            key("ArrowLeft"),
            key("ArrowRight"),
            key("ArrowUp"),
            key("ArrowDown"),
            key("Tab"),
            key("Escape").observing(),
        ],
    );
    h.expect_ok(
        "07-scroll",
        &[step(
            "scroll",
            A::Scroll {
                x: SCROLL_LIST.0,
                y: SCROLL_LIST.1,
                delta_x: 0.0,
                delta_y: 5.0,
            },
        )
        .observing()],
    );
    h.expect_ok(
        "08-drag",
        &[step(
            "drag",
            A::Drag {
                from_x: DRAG_SRC.0,
                from_y: DRAG_SRC.1,
                to_x: DRAG_DST.0,
                to_y: DRAG_DST.1,
                button: PointerButton::Primary,
                duration_ms: 600,
            },
        )
        .observing()],
    );
    h.check_clean("after-primitives");

    // Latency samples (primitive round-trips).
    let lat = h.expect_ok(
        "09-latency",
        &[
            step("observe", A::ObserveScreen),
            click("click", CLICK_BTN),
            step(
                "type",
                A::TypeText {
                    text: "latency".into(),
                    sensitive: false,
                },
            ),
            step(
                "scroll",
                A::Scroll {
                    x: SCROLL_LIST.0,
                    y: SCROLL_LIST.1,
                    delta_x: 0.0,
                    delta_y: -5.0,
                },
            ),
            step(
                "drag",
                A::Drag {
                    from_x: DRAG_SRC.0,
                    from_y: DRAG_SRC.1,
                    to_x: DRAG_DST.0,
                    to_y: DRAG_DST.1,
                    button: PointerButton::Primary,
                    duration_ms: 300,
                },
            ),
        ],
    );
    let ms: Vec<u64> = lat.results.iter().map(|r| r.duration_ms).collect();
    println!(
        "perf: observe={}ms click={}ms type7={}ms scroll={}ms drag300={}ms",
        ms[0], ms[1], ms[2], ms[3], ms[4]
    );

    // Take Control while the agent holds the primary button down.
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        trigger.cancel();
    });
    let held = h.run(
        "10-takeover",
        &[
            step(
                "down",
                A::MouseDown {
                    x: DRAG_SRC.0,
                    y: DRAG_SRC.1,
                    button: PointerButton::Primary,
                },
            ),
            step("hold", A::Wait { duration_ms: 4000 }),
            step(
                "up",
                A::MouseUp {
                    x: DRAG_DST.0,
                    y: DRAG_DST.1,
                    button: PointerButton::Primary,
                },
            ),
        ],
        &cancel,
    );
    t.join().expect("trigger");
    let took = h.registry.take_control();
    println!(
        "takeover: {took:?} executed={}/{}",
        held.steps_executed, held.steps_total
    );
    if took.is_err() {
        h.failures.push(format!("takeover: {took:?}"));
    }
    if held.results.len() > 2 {
        h.failures
            .push("takeover: MouseUp replayed after cancel".into());
    }
    h.check_clean("after-takeover");
    let refused = h.run(
        "11-agent-while-user",
        &[click("click", CLICK_BTN)],
        &CancellationToken::new(),
    );
    if refused.results.first().is_some_and(|r| r.success) {
        h.failures
            .push("agent acted while user owns control".into());
    }
    if took.is_ok() {
        h.registry.return_control().expect("return control");
    }
    h.expect_ok("12-after-return", &[click("click", CLICK_BTN).observing()]);

    // Pause mid-action: the running Wait is interrupted, the Click after
    // it must never replay; then a NEW action succeeds after resume.
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        trigger.cancel();
    });
    let paused = h.run(
        "13-pause",
        &[
            step("wait", A::Wait { duration_ms: 4000 }),
            click("click", CLICK_BTN),
        ],
        &cancel,
    );
    t.join().expect("trigger");
    let state = h.registry.pause().expect("pause");
    println!(
        "pause: state={state:?} outcome={:?} executed={}/{}",
        paused.results.first().map(|r| r.outcome.clone()),
        paused.steps_executed,
        paused.steps_total
    );
    if paused.results.len() != 1 {
        h.failures
            .push("pause: action replayed or not interrupted".into());
    }
    std::thread::sleep(Duration::from_secs(3));
    h.registry.resume().expect("resume");
    if !wait_guest(&mut h.registry, Duration::from_secs(30)) {
        h.failures
            .push("pause: guest not ready after short resume".into());
    }
    h.expect_ok("14-after-resume", &[click("click", CLICK_BTN).observing()]);

    reconnect_check(&mut h);
    finish(h);
}

fn finish(mut h: Harness) {
    h.check_clean("final");

    std::thread::sleep(Duration::from_secs(5));
    match h.registry.guest_info_request(Duration::from_secs(5)) {
        Ok(info) => {
            let rss: Vec<String> = info
                .process_rss
                .iter()
                .map(|p| format!("{}={}KB", p.name, p.rss_kb))
                .collect();
            println!(
                "perf: guest mem_total={:?}MB mem_available={:?}MB cpu_jiffies={:?} rss=[{}]",
                info.mem_total_mb,
                info.mem_available_mb,
                info.cpu_jiffies.as_ref().map(|j| (j.busy, j.total)),
                rss.join(" ")
            );
            std::thread::sleep(Duration::from_secs(10));
            if let (Ok(later), Some(a)) = (
                h.registry.guest_info_request(Duration::from_secs(5)),
                info.cpu_jiffies.as_ref(),
            ) {
                if let Some(b) = later.cpu_jiffies.as_ref() {
                    let busy = b.busy.saturating_sub(a.busy) as f64;
                    let total = b.total.saturating_sub(a.total).max(1) as f64;
                    println!(
                        "perf: guest idle cpu over 10s = {:.1}%",
                        100.0 * busy / total
                    );
                }
            }
        }
        Err(e) => println!("perf: guest info unavailable: {e}"),
    }
    println!("host pointer at end={}", host_pointer());

    h.registry.stop().expect("stop");
    if h.failures.is_empty() {
        println!("E2E DONE");
    } else {
        for f in &h.failures {
            eprintln!("E2E FAIL: {f}");
        }
        std::process::exit(2);
    }
}

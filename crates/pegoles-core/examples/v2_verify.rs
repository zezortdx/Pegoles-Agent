//! Phase 5.1 hardware verification: full Core stack against Image v2.
//!
//! ```sh
//! PEGOLES_IMAGE_ID=pegoles-base-0.2 cargo run -p pegoles-computer --example v2_verify
//! ```
//!
//! Boots v2, waits for GuestReady + graphical Ready, prints advertised
//! capabilities, then runs the deterministic demo script through the REAL
//! executor (policy → arbiter → guest channel) printing every lifecycle
//! event. Exit 0 only if the success-gate path completes; exit 2 with a
//! precise diagnosis otherwise. No model, no UI, no fakes.

use pegoles_computer::platform::BackendKind;
use pegoles_core::{pegoles_demo_script, CancellationToken, ComputerRegistry, EventBus};
use pegoles_policy::PolicyContext;
use pegoles_protocol::GuestRuntimeState;
use std::time::{Duration, Instant};

fn main() {
    let image_id = std::env::var("PEGOLES_IMAGE_ID").unwrap_or_else(|_| "pegoles-base-0.1".into());
    println!("image: {image_id}");
    let bus = EventBus::new();
    let mut registry = ComputerRegistry::with_backend_kind(bus, BackendKind::MacOSVirtualization);
    let id = registry.create_default().expect("create");
    println!("computer: {id}");
    registry.start().expect("start");

    // GuestReady + graphical Ready (weston session) with a generous budget.
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut graphical_ready = false;
    loop {
        registry.pump();
        let guest = registry.guest_state();
        let session = registry.graphical_session();
        if guest == GuestRuntimeState::Ready
            && matches!(
                session.state,
                pegoles_protocol::GraphicalSessionState::Ready
            )
        {
            graphical_ready = true;
            println!(
                "graphical ready: {:?} {}x{}",
                session.compositor,
                session.width_px.unwrap_or(0),
                session.height_px.unwrap_or(0),
            );
            break;
        }
        if Instant::now() > deadline {
            eprintln!(
                "V2 VERIFY FAIL: guest={guest:?} graphical={:?}",
                session.state
            );
            std::process::exit(2);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(graphical_ready);

    println!("input_available: {}", registry.input_available());
    let st = registry.input_status();
    println!(
        "agent_busy={} pressed_clean={} audit={}",
        st.agent_busy, st.pressed_clean, st.audit_len
    );
    if !registry.input_available() {
        eprintln!("V2 VERIFY FAIL: input plane unavailable on v2 image");
        std::process::exit(2);
    }

    // Full deterministic demo through the real executor.
    let task = pegoles_protocol::TaskId::new();
    let steps = pegoles_demo_script();
    println!("demo steps: {}", steps.len());
    let (report, events) = registry.run_script(
        task,
        &steps,
        &PolicyContext::default(),
        &CancellationToken::new(),
    );
    let mut counts = std::collections::HashMap::new();
    for e in &events {
        let t = serde_json::to_value(e).expect("event serializes");
        *counts
            .entry(t["type"].as_str().unwrap_or("?").to_string())
            .or_insert(0) += 1;
        println!("event: {}", t["type"].as_str().unwrap_or("?"));
    }
    println!("event histogram: {counts:?}");
    for (i, r) in report.results.iter().enumerate() {
        println!(
            "step {i}: success={} outcome={:?} {}ms msg={}",
            r.success, r.outcome, r.duration_ms, r.message
        );
    }
    println!(
        "report: {}/{} executed, aborted_at={:?}",
        report.steps_executed, report.steps_total, report.aborted_at
    );
    if report.aborted_at.is_some() || report.steps_executed != report.steps_total {
        eprintln!("V2 VERIFY FAIL: demo aborted");
        std::process::exit(2);
    }
    // State assertions: released control, clean pressed, audit rows.
    assert!(!registry.input_status().agent_busy, "control not released");
    assert!(registry.input_status().pressed_clean, "pressed state dirty");
    println!("state after demo: control released, pressed clean");

    registry.stop().expect("stop");
    println!("V2 VERIFY DONE");
}

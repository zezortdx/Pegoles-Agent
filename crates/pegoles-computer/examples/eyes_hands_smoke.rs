//! Phase 5 hardware smoke (Apple Silicon, manual run only).
//!
//! ```sh
//! cargo run -p pegoles-computer --example eyes_hands_smoke
//! ```
//!
//! Boots a real computer, waits for GuestReady, then exercises the
//! Eyes & Hands negotiation path against the sealed guest runtime:
//! - `input_available` (expected: false on v0.1 guests — no caps)
//! - `input_execute` (expected: honest UnsupportedOperation naming the
//!   missing capability — proves the round-trip path, not a pretend)
//! - `input_capture_frame` (expected: same honest refusal)
//! - control/takeover flows stay untouched (no display in this harness)
//!
//! Prints real latency numbers for PERFORMANCE.md. Never fails the suite
//! (example, not a test): exit 0 prints results, exit 2 on HW faults.

use pegoles_computer::coords::GuestPoint;
use pegoles_computer::{pegoles_data_dir, ComputerBackend, InputOp, MacOSVirtualizationBackend};
use pegoles_protocol::PointerButton;
use std::time::{Duration, Instant};

fn main() {
    let data = pegoles_data_dir();
    let scratch = std::env::temp_dir().join(format!("pg-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch");
    println!("images: {}", data.join("images").display());
    println!("computers: {}", scratch.display());

    let mut backend =
        MacOSVirtualizationBackend::new(data.join("images"), scratch).expect("macos backend");
    let t0 = Instant::now();
    let id = backend
        .create(pegoles_computer::default_config())
        .expect("create");
    println!("created {id} in {:?}", t0.elapsed());

    let t0 = Instant::now();
    backend.start().expect("start");
    println!("start accepted in {:?}", t0.elapsed());

    // Wait for GuestReady (boot ~6-8 s + margin).
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let _ = backend.poll_guest();
        if backend.guest_state() == pegoles_protocol::GuestRuntimeState::Ready {
            break;
        }
        if Instant::now() > deadline {
            eprintln!(
                "SMOKE FAIL: guest never Ready ({:?})",
                backend.guest_state()
            );
            std::process::exit(2);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("GuestReady (state={:?})", backend.guest_state());

    // Baseline round-trip latency on the same channel input will use.
    let t0 = Instant::now();
    match backend.guest_ping(Duration::from_secs(10)) {
        Ok(latency) => println!("guest ping: {latency} ms (call took {:?})", t0.elapsed()),
        Err(e) => println!("guest ping FAILED: {e}"),
    }

    // Eyes & Hands negotiation against the sealed v0.1 guest.
    println!("input_available: {}", backend.input_available());
    let caps = backend.input_capabilities();
    println!(
        "input caps: pointer={} keyboard={} screenshot={} kind={:?}",
        caps.pointer, caps.keyboard, caps.screenshot, caps.kind
    );
    let out = backend.input_execute(
        "smoke-1",
        &InputOp::Move {
            point: GuestPoint { x: 720, y: 450 },
        },
    );
    println!(
        "input_execute(move): ok={} error={:?} latency={}ms",
        out.ok, out.error, out.latency_ms
    );
    match backend.input_capture_frame("smoke-frame", Duration::from_secs(10)) {
        Ok(frame) => println!(
            "capture: {}x{} {} bytes (UNEXPECTED on v0.1 guest)",
            frame.meta.width_px,
            frame.meta.height_px,
            frame.bytes.len()
        ),
        Err(e) => println!("capture refused (expected on v0.1 guest): {e}"),
    }
    // Button mapping sanity (no backend involved).
    let _ = PointerButton::Primary;

    let t0 = Instant::now();
    backend.stop().expect("stop");
    println!("stopped in {:?}", t0.elapsed());
    println!("SMOKE DONE");
}

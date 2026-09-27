//! Guest memory-integrity repro (Apple silicon, manual run only).
//!
//! Boots a computer from the pinned image and alternates file-heavy shell
//! work in the guest terminal with idle periods while frames are captured
//! (like an agent run). Stops at the first kernel oops or panic in the
//! guest serial log, or after the given number of cycles.
//!
//! ```sh
//! cargo build --release -p pegoles-computer --example guest_memory_repro
//! cp <signed pegoles-vm-host> target/release/examples/
//! ./target/release/examples/guest_memory_repro [cycles] [idle_secs]
//! ```
//! Exit 0: no guest fault; exit 1: guest oops/panic (log excerpt printed).

use pegoles_computer::{pegoles_data_dir, ComputerBackend, InputOp, MacOSVirtualizationBackend};
use std::time::{Duration, Instant};

fn guest_fault(log: &std::path::Path) -> Option<String> {
    let text = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&text);
    text.lines()
        .find(|l| l.contains("Internal error") || l.contains("Kernel panic") || l.contains("Oops"))
        .map(str::to_string)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cycles: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6);
    let idle = Duration::from_secs(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(90));
    let data = pegoles_data_dir();
    let scratch = std::env::temp_dir().join(format!("pg-memrepro-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch");
    let mut backend =
        MacOSVirtualizationBackend::new(data.join("images"), scratch.clone()).expect("backend");
    backend
        .create(pegoles_computer::default_config())
        .expect("create");
    backend.start().expect("start");
    let deadline = Instant::now() + Duration::from_secs(120);
    while backend.guest_state() != pegoles_protocol::GuestRuntimeState::Ready {
        let _ = backend.poll_guest();
        assert!(Instant::now() < deadline, "guest never ready");
        std::thread::sleep(Duration::from_millis(500));
    }
    // Input needs the guest display as well as the runtime.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !backend.input_available() {
        let _ = backend.poll_guest();
        assert!(Instant::now() < deadline, "input never available");
        std::thread::sleep(Duration::from_millis(500));
    }
    // The guest learns its display size from the first capture.
    let first = backend.input_capture_frame("first", Duration::from_secs(20));
    assert!(first.is_ok(), "first capture failed");
    let log = backend.serial_log_path().expect("serial log");
    println!("ready; serial log {}", log.display());
    let t0 = Instant::now();
    let mut result = 0;
    'outer: for c in 1..=cycles {
        let cmd = std::env::var("REPRO_CMD").unwrap_or_else(|_| {
            format!(
                "cd ~ && for i in $(seq 1 300); do mkdir -p w{c}/$i && echo pegoles > w{c}/$i/f; done; \
                 cp -r w{c} c{c} && chmod -R g-w c{c} && rm -rf c{c}; sync; ls w{c} | wc -l"
            )
        });
        let out = backend.input_execute(&format!("t{c}"), &InputOp::TypeText { text: cmd });
        let _ = backend.input_execute(
            &format!("k{c}"),
            &InputOp::KeyPress {
                key: "Enter".into(),
            },
        );
        println!(
            "[{:>4}s] cycle {c}: typed ok={} err={:?}",
            t0.elapsed().as_secs(),
            out.ok,
            out.error
        );
        let until = Instant::now() + idle;
        let mut n = 0;
        while Instant::now() < until {
            let _ = backend.poll_guest();
            let frame = backend.input_capture_frame(&format!("f{c}-{n}"), Duration::from_secs(10));
            if let (Ok(frame), Ok(path)) = (frame, std::env::var("REPRO_FRAME_OUT")) {
                let _ = std::fs::write(path, &frame.bytes);
            }
            n += 1;
            if let Some(line) = guest_fault(&log) {
                println!(
                    "[{:>4}s] GUEST FAULT in cycle {c}: {line}",
                    t0.elapsed().as_secs()
                );
                result = 1;
                break 'outer;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        println!(
            "[{:>4}s] cycle {c}: {n} captures, no fault",
            t0.elapsed().as_secs()
        );
    }
    let _ = backend.stop();
    if std::env::var("REPRO_KEEP").is_ok() {
        println!("kept computer under {}", scratch.display());
    } else {
        let _ = backend.destroy();
        let _ = std::fs::remove_dir_all(&scratch);
    }
    println!(
        "{}",
        if result == 0 {
            "NO GUEST FAULT"
        } else {
            "GUEST FAULT"
        }
    );
    std::process::exit(result);
}

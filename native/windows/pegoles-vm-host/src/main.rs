//! pegoles-vm-host.exe — Pegoles' unprivileged VM helper on Windows.
//!
//! Speaks exactly the macOS helper's JSONL command set
//! (`pegoles_computer::vmhost_proto`) on stdin/stdout, so the shared
//! engine (`NativeHelperBackend`) and the guest session run unchanged.
//! Underneath:
//!
//! - lifecycle goes to the `PegolesVmBroker` service over its named pipe
//!   (the only component with Host Compute System rights);
//! - the guest channel is an AF_HYPERV connection from THIS process to the
//!   guest runtime's privileged listener — guest bytes never reach the
//!   privileged service;
//! - COM1 arrives on a pipe this process owns and goes to a bounded
//!   `logs/serial.log`.
//!
//! No shell, no host files beyond the computer's own log, no network.
//! When stdin closes (the app quit or crashed) the broker connection
//! closes with it and the broker terminates the VMs (lease).

mod dispatch;
#[cfg(any(windows, test))]
mod framer;
#[cfg(windows)]
mod win;

use std::io::{BufRead, Read, Write};
use std::sync::{Arc, Mutex};

/// Serialized, line-atomic stdout shared by responses and async events.
pub struct Out(Mutex<Box<dyn Write + Send>>);

impl Out {
    pub fn new(writer: Box<dyn Write + Send>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(writer)))
    }

    pub fn line(&self, value: &impl serde::Serialize) {
        let Ok(mut text) = serde_json::to_string(value) else {
            return;
        };
        text.push('\n');
        if let Ok(mut out) = self.0.lock() {
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
    }
}

/// Longest request line accepted from the engine.
const MAX_REQUEST_BYTES: usize = 256 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("--version" | "-V")) {
        println!("pegoles-vm-host-windows {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let out = Out::new(Box::new(std::io::stdout()));
    #[cfg(windows)]
    let machine = win::WinMachine::new(out.clone());
    #[cfg(not(windows))]
    let machine = dispatch::Unavailable;
    let mut host = dispatch::Host::new(machine, out.clone());
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let mut limited = (&mut reader).take(MAX_REQUEST_BYTES as u64 + 1);
        match limited.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > MAX_REQUEST_BYTES {
            // Oversized: the engine never sends this; stop rather than guess.
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let response = host.handle_line(text);
        out.line(&response);
    }
    host.shutdown();
}

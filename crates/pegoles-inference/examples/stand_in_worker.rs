//! A stand-in for `pegoles-llm-worker`, used by the Windows sandbox tests.
//!
//! It speaks just enough of protocol v1 (`hello`, `shutdown`) and answers
//! `probe` by trying what a compromised worker would try: read a file it
//! was not given, reach the network (loopback included), start a process.
//! Inside the AppContainer and job object every attempt must fail; the
//! test runs it unconfined first to prove the probes work.

use std::io::{BufRead, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{json, Value};

fn probe(request: &Value) -> Value {
    let file_read = request["file"]
        .as_str()
        .is_some_and(|path| std::fs::read(path).is_ok());
    let network = request["addr"]
        .as_str()
        .and_then(|addr| addr.parse::<SocketAddr>().ok())
        .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok());
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let process = std::process::Command::new(format!(r"{system_root}\System32\cmd.exe"))
        .args(["/c", "exit 0"])
        .status()
        .is_ok();
    json!({ "file_read": file_read, "network": network, "process": process })
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = request["id"].clone();
        let (mut reply, stop) = match request["op"].as_str() {
            Some("hello") => (
                json!({
                    "protocol": 1, "worker": "pegoles-llama", "worker_version": "stand-in",
                    "backend": "CPU", "device": "", "gpu": false
                }),
                false,
            ),
            Some("probe") => (probe(&request), false),
            Some("shutdown") => (json!({}), true),
            _ => (json!({}), false),
        };
        reply["v"] = json!(1);
        reply["id"] = id;
        reply["ok"] = json!(true);
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
        if stop {
            break;
        }
    }
}

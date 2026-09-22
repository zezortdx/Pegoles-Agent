//! Validate a guest kernel config for Pegoles transports.
//!
//! Usage: `cargo run -p pegoles-computer --example check_guest_kernel --
//!   /path/to/kernel-config`
//! Reads a `/boot/config-*` (or decompressed `/proc/config.gz`) text and
//! reports vsock transport support as JSON. Exit 0 always on parseable
//! input; the `universal_ready` field is the gate (exit 2 when false and
//! `--strict-universal` is passed).

use pegoles_computer::check_kernel_config;
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let strict = args.iter().any(|a| a == "--strict-universal");
    let path = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("usage: check_guest_kernel [--strict-universal] <kernel-config>");
            std::process::exit(2);
        });
    let mut text = String::new();
    std::fs::File::open(&path)
        .unwrap_or_else(|e| {
            eprintln!("cannot open {path}: {e}");
            std::process::exit(2);
        })
        .read_to_string(&mut text)
        .unwrap_or_else(|e| {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        });
    let caps = check_kernel_config(&text);
    println!(
        "{}",
        serde_json::json!({
            "vsock": format!("{:?}", caps.vsock),
            "virtio_vsock": format!("{:?}", caps.virtio_vsock),
            "hyperv_vsock": format!("{:?}", caps.hyperv_vsock),
            "virtio_ready": caps.virtio_ready(),
            "hyperv_ready": caps.hyperv_ready(),
            "universal_ready": caps.universal_ready(),
        })
    );
    if strict && !caps.universal_ready() {
        eprintln!("universal image gate FAILED: needs virtio + hyperv vsock");
        std::process::exit(2);
    }
}

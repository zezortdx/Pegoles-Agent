//! pegoles-broker.exe — the privileged half of Pegoles on Windows.
//!
//! Verbs (all fixed; nothing is forwarded to a shell):
//!
//! - `service`: run as the `PegolesVmBroker` Windows service (started by
//!   the Service Control Manager on demand). Serves the named pipe in
//!   `pegoles-broker-proto` and owns every Host Compute System call.
//! - `run-foreground`: the same server in a console (CI and debugging;
//!   needs an elevated prompt).
//! - `install-service` / `uninstall-service`: called by the installer
//!   (already elevated) to register or remove the service.
//! - `enable-virtualization`: turn on the Virtual Machine Platform
//!   feature with DISM. Run elevated from onboarding after the person
//!   agreed; exits 0 (on), 3010 (on after a restart) or 1 (failed).
//! - `--version`.

#[cfg(windows)]
mod win;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--version") {
        println!(
            "pegoles-broker {} (protocol {})",
            env!("CARGO_PKG_VERSION"),
            pegoles_broker_proto::PROTOCOL_VERSION
        );
        return;
    }
    #[cfg(windows)]
    {
        std::process::exit(win::main(&args));
    }
    #[cfg(not(windows))]
    {
        eprintln!("pegoles-broker runs on Windows only");
        std::process::exit(2);
    }
}

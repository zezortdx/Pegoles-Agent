//! `pegoles-egress-forwarder [--policy-file <path>]`

use std::path::PathBuf;
use std::process::ExitCode;

use pegoles_egress_forwarder::{Config, DEFAULT_POLICY_FILE};

fn parse_args() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut policy = PathBuf::from(DEFAULT_POLICY_FILE);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--policy-file" => {
                policy = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--policy-file needs a path")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(policy)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    use pegoles_egress_forwarder::{server, vsock::VsockListener};

    let policy = match parse_args() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pegoles-egress-forwarder: {e}");
            return ExitCode::from(2);
        }
    };
    let listener = match VsockListener::bind(pegoles_egress_proto::VSOCK_PORT) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("pegoles-egress-forwarder: cannot listen on vsock: {e}");
            return ExitCode::FAILURE;
        }
    };
    server::serve(&listener, &Config::production(policy));
    ExitCode::SUCCESS
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = (parse_args(), Config::production(PathBuf::new()));
    eprintln!("pegoles-egress-forwarder: Linux guest only (AF_VSOCK)");
    ExitCode::FAILURE
}

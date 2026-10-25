//! Guest end of the egress channel (docs/EGRESS.md).
//!
//! Listens on AF_VSOCK 4051 for the host. After a valid HELLO it installs the
//! per-session CA into Chromium's managed policy file, answers HELLO_ACK and
//! only then opens TCP 127.0.0.1:3128 for Chromium; every TCP connection
//! becomes one mux stream. When the channel ends, the listener and all
//! connections are closed and the policy file is reset.
//!
//! SECURITY: the host channel is treated as hostile input (every byte is
//! validated by `pegoles-egress-proto` or `der`), memory per stream is bounded
//! by the mux window, nothing here logs payloads, and no panic path depends
//! on peer data.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// Diagnostics go to stderr (journald). Never pass payload bytes.
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("pegoles-egress-forwarder: {}", format_args!($($arg)*))
    };
}

pub mod der;
pub mod policy_file;
pub mod server;
pub mod session;
pub mod transport;
#[cfg(target_os = "linux")]
pub mod vsock;

/// The one file the forwarder user may write (see the systemd unit).
pub const DEFAULT_POLICY_FILE: &str = "/etc/chromium/policies/managed/pegoles-egress-ca.json";

#[derive(Clone, Debug)]
pub struct Config {
    /// Managed-policy file carrying the session CA (written in place).
    pub policy_file: PathBuf,
    /// Where Chromium's proxy listener binds once the handshake is done.
    pub proxy_addr: SocketAddr,
    /// Test hook: receives the bound address after the handshake.
    pub listening: Option<Sender<SocketAddr>>,
}

impl Config {
    pub fn production(policy_file: PathBuf) -> Config {
        Config {
            policy_file,
            proxy_addr: SocketAddr::from((
                std::net::Ipv4Addr::LOCALHOST,
                pegoles_egress_proto::GUEST_PROXY_PORT,
            )),
            listening: None,
        }
    }
}

#[cfg(test)]
mod tests;

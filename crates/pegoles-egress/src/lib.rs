//! Pegoles egress: the host side of the agent computer's internet access
//! (docs/EGRESS.md). A session speaks the mux protocol
//! (`pegoles-egress-proto`) over one byte stream to the guest and serves an
//! HTTP proxy behind it: immutable policy, TLS interception with a
//! per-session CA, response inspection, audit events.
//!
//! ```text
//! let (sink, mut events) = pegoles_egress::audit::channel();
//! let session = EgressSession::start(stream, Mode::allowlist(["example.com"])?, sink).await?;
//! // ... task runs ...
//! session.shutdown().await; // or just drop it
//! ```
pub mod audit;
pub mod inspect;
mod mux;
pub mod policy;
mod proxy;
mod session;
pub mod threat;
pub mod tls;

pub use audit::{AuditEvent, AuditSink};
pub use mux::SessionEnd;
pub use policy::{AllowlistError, Decision, DenyReason, Domain, Mode, ModeKind};
pub use session::{EgressSession, StartError};

#[cfg(test)]
mod testutil;

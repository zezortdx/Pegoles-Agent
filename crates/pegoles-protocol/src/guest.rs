//! Guest runtime state: is the OS inside the VM ready, not just the VM.
//!
//! `ComputerState::Running` means the hypervisor started the VM.
//! `GuestRuntimeState::Ready` additionally requires a completed vsock
//! handshake (connection + valid GuestHello + compatible version + Ready).
//! The two are tracked and displayed separately, never conflated.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestRuntimeState {
    /// No VM/computer, or backend without guest support (e.g. Mock).
    Unavailable,
    /// VM running, no vsock connection yet.
    Waiting,
    /// Vsock connected, handshake in flight.
    Connecting,
    /// Handshake complete: hello accepted, version compatible, ready seen.
    Ready,
    /// Channel lost after having existed. Reconnect may follow.
    Disconnected,
    /// Guest speaks an incompatible protocol version. Never proceed silently.
    Incompatible,
    /// Handshake timeout or protocol violation.
    Error,
}

impl GuestRuntimeState {
    /// Whether Core may consider the guest usable.
    pub fn is_ready(&self) -> bool {
        matches!(self, GuestRuntimeState::Ready)
    }
}

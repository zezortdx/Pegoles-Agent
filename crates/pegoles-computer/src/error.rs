use pegoles_protocol::{ComputerId, ComputerState, SnapshotId};
use thiserror::Error;

/// All failures a computer backend can report.
///
/// SECURITY NOTE: no variant here may carry a host shell command or a host
/// path to execute. Error strings are diagnostics only.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ComputerError {
    #[error("no computer created yet")]
    NotCreated,
    #[error("computer already created: {0}")]
    AlreadyCreated(ComputerId),
    #[error("invalid transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ComputerState,
        to: ComputerState,
    },
    #[error("operation requires state {expected:?}, current is {actual:?}")]
    WrongState {
        expected: ComputerState,
        actual: ComputerState,
    },
    #[error("unknown snapshot: {0:?}")]
    UnknownSnapshot(SnapshotId),
    /// The real VM backend cannot run on this OS/arch (e.g. not macOS/arm64).
    /// Never silently fall back to host execution; surface this instead.
    #[error("unsupported platform for real VM backend: {0}")]
    UnsupportedPlatform(String),
    /// Right OS family, wrong architecture for this backend's current target
    /// (e.g. WindowsHcsBackend on ARM64 while x86_64 ships first).
    #[error("unsupported architecture: {0}")]
    UnsupportedArchitecture(String),
    /// Host fails a readiness gate that setup could fix (Hyper-V disabled,
    /// socket service unregistered, …). Surfaces per-item setup steps,
    /// never a bare "unsupported".
    #[error("setup required: {0}")]
    SetupRequired(String),
    /// Operation the backend does not implement in this phase
    /// (e.g. snapshot/restore on the real VM backend).
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),
    /// Operation is specified for a future backend phase but explicitly
    /// not implemented yet (e.g. WindowsHcsBackend lifecycle). Never
    /// returned silently; never a Mock fallback.
    #[error("backend feature not implemented: {0}")]
    BackendFeatureNotImplemented(String),
    /// The native VM host process exited or the IPC channel broke.
    /// Core must not keep displaying Running after this.
    #[error("native VM host disconnected: {0}")]
    BackendDisconnected(String),
    /// No verified base image available; run image preparation first.
    #[error("computer image missing: {0}")]
    ImageMissing(String),
    /// Downloaded image failed checksum verification; it was rejected.
    #[error("computer image verification failed: {0}")]
    ImageVerificationFailed(String),
    /// VM configuration outside allowed bounds.
    #[error("invalid VM configuration: {0}")]
    InvalidConfig(String),
    /// No guest vsock connection established right now.
    #[error("guest unavailable: {0}")]
    GuestUnavailable(String),
    /// No display on this host/backend, or the display cannot perform
    /// the request (e.g. headless computer). Never faked.
    #[error("display unavailable: {0}")]
    DisplayUnavailable(String),
    #[error("backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, ComputerError>;

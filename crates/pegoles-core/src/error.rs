use pegoles_computer::ComputerError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("invalid task transition: {from:?} -> {to:?}")]
    InvalidTaskTransition {
        from: pegoles_protocol::TaskStatus,
        to: pegoles_protocol::TaskStatus,
    },
    #[error("computer error: {0}")]
    Computer(#[from] ComputerError),
    #[error("no computer available")]
    NoComputer,
    #[error("no boot log available (mock backend has no guest console)")]
    NoBootLog,
    #[error("event bus lagged")]
    BusLagged,
    /// Untrusted framebuffer geometry from the UI failed validation.
    #[error("invalid display geometry: {0:?}")]
    InvalidGeometry(pegoles_computer::GeometryError),
    /// Take Control refused: the display is not ready to be controlled.
    #[error("control unavailable: {0}")]
    ControlUnavailable(String),
    #[error("invalid task title: {0}")]
    InvalidTaskTitle(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

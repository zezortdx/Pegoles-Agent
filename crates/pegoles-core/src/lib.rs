//! Pegoles Core: coordinates tasks, computers, actions, policy, events.
//!
//! Phase 1 is in-memory only (no SQLite). UI never touches internals
//! directly; it goes through this crate (via Tauri commands) and
//! subscribes to [`AgentEvent`]s.

pub mod display;
#[cfg(test)]
mod display_tests;
pub mod egress;
pub mod error;
pub mod events;
pub mod input;
pub mod registry;
pub mod tasks;
pub mod viewport;

pub use display::{
    default_display_config, ComputerView, DisplayBounds, GeometryOutcome, GEOMETRY_EPSILON_PX,
};
pub use egress::{
    validate_internet, EgressHandshake, EgressKill, EgressOpening, EgressStatus, HANDSHAKE_TIMEOUT,
};
pub use error::{CoreError, Result};
pub use events::EventBus;
pub use input::{
    pegoles_demo_script, AuditEntry, AuditLog, CancellationToken, ControlArbiter, InputStatus,
    ScriptReport, ScriptStep, WaitTicket,
};
pub use pegoles_computer::platform::BackendKind;
pub use registry::{default_backend_kind, BootLog, ComputerRegistry};
pub use tasks::{TaskManager, MAX_TASK_TITLE_CHARS};
pub use viewport::{derive_viewport, ViewportFacts, ViewportIssue, ViewportView};

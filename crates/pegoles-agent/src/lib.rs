//! Pegoles agent orchestrator.
//!
//! ```text
//! objective ─▶ runner ─▶ planner.next() ─▶ PlannedCall(steps)
//!                 │                              │
//!                 │        typed ComputerAction ◀┘
//!                 ▼
//!          AgentComputer::act ─▶ Core executor ─▶ Pegoles Policy ─▶ guest
//! ```
//!
//! The planner (a model or a script) only proposes typed actions. The
//! runner enforces budgets and cancellation; Core enforces policy and
//! control ownership. Nothing here can reach the host: the only effect
//! of a planned call is a computer action inside the VM.

pub mod anthropic;
pub mod core_computer;
pub mod planner;
pub mod runner;
pub mod scripted;

pub use core_computer::{CoreAccess, CoreComputer};
pub use planner::{
    CallOutcome, CallOutput, PlannedCall, Planner, PlannerError, PlannerTurn, Screenshot, Step,
};
pub use runner::{run_task, AgentComputer, RunEnd, RunLimits, RunReport};
pub use scripted::ScriptedPlanner;

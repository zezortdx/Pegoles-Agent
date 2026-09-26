//! Pegoles Guard: deterministic policy evaluation.
//!
//! SECURITY INVARIANTS:
//! - No AI/heuristics decide anything here: a pure, exhaustive match on
//!   typed actions. A model can request; only this code decides.
//! - Every action in the vocabulary acts inside the isolated VM. Shape
//!   checks (unit-square coordinates, caps, key vocabulary) deny
//!   malformed or out-of-range input.
//! - Allow → risk Low; Deny → risk Blocked. `RequireApproval` is reserved
//!   for future capabilities with effects outside the VM and is treated
//!   as a denial by the executor until approval grants exist.

pub mod engine;

pub use engine::{evaluate, PolicyContext};

//! Pegoles Guard (Phase 1): deterministic policy evaluation.
//!
//! SECURITY INVARIANTS:
//! - No AI/heuristics here. Pure deterministic match on structured actions.
//! - `Shell`/`ReadFile`/`WriteFile` are guest-scoped by construction.
//! - Anything resembling host access or credential access is denied.
//!
//! Risk mapping: Low -> Allow, Medium -> RequireApproval,
//! High -> RequireApproval (conservative), Blocked -> Deny.

pub mod engine;

pub use engine::{evaluate, PolicyContext};

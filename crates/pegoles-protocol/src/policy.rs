//! Policy primitives: capabilities, risk, decisions, approvals.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::actions::ActionRequest;
use crate::ids::TaskId;

/// What a component is allowed to do. Kept intentionally small for Phase 1.
/// A future explicit `HostBridge` capability would gate any host access
/// (see docs/SECURITY.md); it is NOT granted anywhere in Phase 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ComputerCreate,
    ComputerControl,
    ComputerObserve,
    FileWriteWorkspace,
    ShellExec,
}

/// Fine-grained permission checked by UI / core before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Allowed,
    NeedsApproval,
    Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVerdict {
    pub decision: Decision,
    pub risk: RiskLevel,
    pub reason: String,
}

/// Human (or future mobile approver) decision for a gated action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub task_id: TaskId,
    pub action: ActionRequest,
    pub reason: String,
    pub requested_at: DateTime<Utc>,
}

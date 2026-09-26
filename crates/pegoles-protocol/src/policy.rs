//! Policy primitives: risk and decisions.

use serde::{Deserialize, Serialize};

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
    /// Reserved for capabilities with effects outside the VM. No action
    /// in the current vocabulary produces it; executors treat it as a
    /// denial until an approval grant exists (fail closed).
    RequireApproval,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVerdict {
    pub decision: Decision,
    pub risk: RiskLevel,
    pub reason: String,
}

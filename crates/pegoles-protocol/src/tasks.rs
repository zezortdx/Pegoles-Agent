//! Tasks shared between core and UI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::TaskId;

/// Lifecycle of an agent task. Transitions are enforced by `pegoles-core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AgentTask {
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: TaskId::new(),
            title: title.into(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
        }
    }
}

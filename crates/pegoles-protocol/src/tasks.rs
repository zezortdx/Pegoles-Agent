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

/// How much of the internet a task may reach (docs/EGRESS.md). Chosen per
/// task, `off` by default; the computer itself never has a network device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InternetMode {
    #[default]
    Off,
    /// Only the domains the person listed for this task.
    Allowlist,
    /// Any public site, minus the always-blocked layer.
    OpenWeb,
}

impl InternetMode {
    pub fn as_str(self) -> &'static str {
        match self {
            InternetMode::Off => "off",
            InternetMode::Allowlist => "allowlist",
            InternetMode::OpenWeb => "open_web",
        }
    }
}

/// A task's internet setting as the UI sends it: a mode and domain
/// strings, nothing else (no URLs, paths or endpoints).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternetAccess {
    #[serde(default)]
    pub mode: InternetMode,
    #[serde(default)]
    pub domains: Vec<String>,
}

impl InternetAccess {
    pub fn off() -> Self {
        Self::default()
    }

    pub fn is_off(&self) -> bool {
        self.mode == InternetMode::Off
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Validated and normalized by Core when the task is created.
    #[serde(default)]
    pub internet: InternetAccess,
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
            internet: InternetAccess::off(),
        }
    }
}

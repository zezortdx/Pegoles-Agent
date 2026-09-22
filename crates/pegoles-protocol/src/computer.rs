//! Computer state + config. Paths here are always guest-virtual paths.

use serde::{Deserialize, Serialize};

use crate::display::DisplayConfig;
use crate::ids::ComputerId;

/// A path inside Pegoles Computer (the guest), never the host.
/// The `VirtualPath` wrapper exists so host paths cannot be confused
/// with guest paths at the type level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VirtualPath(pub String);

impl VirtualPath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for VirtualPath {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerState {
    /// Backend allocated but not started.
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
    /// Non-recoverable backend error; `reset()` may recover.
    Error,
}

impl ComputerState {
    /// Whether user-initiated agent actions can run.
    pub fn can_act(&self) -> bool {
        matches!(self, ComputerState::Running)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerConfig {
    pub vcpus: u8,
    pub memory_mb: u32,
    pub disk_gb: u32,
    /// Root of the agent workspace *inside the guest*.
    pub workspace_root: VirtualPath,
    /// Guest framebuffer. `None` = headless computer (no graphics device,
    /// no keyboard/pointer devices). Older serialized configs deserialize
    /// as headless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayConfig>,
}

impl Default for ComputerConfig {
    fn default() -> Self {
        Self {
            vcpus: 2,
            memory_mb: 2048,
            disk_gb: 20,
            workspace_root: VirtualPath::new("/home/pegoles/workspace"),
            display: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerInfo {
    pub id: ComputerId,
    pub state: ComputerState,
    pub config: ComputerConfig,
}

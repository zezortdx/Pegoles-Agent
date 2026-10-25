//! Agent events: the observability backbone (UI, logs, audit, mobile).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::actions::{ActionRequest, ActionResult};
use crate::computer::ComputerState;
use crate::display::{ControlOwner, ObservedFrameMeta};
use crate::ids::{ActionId, ComputerId, TaskId};
use crate::policy::PolicyVerdict;
use crate::tasks::{InternetMode, TaskStatus};

/// What an `AgentMessage` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    /// What the agent is doing or noticed (between actions).
    Progress,
    /// The agent's final account of the task.
    Summary,
    /// Why the run stopped without finishing.
    Error,
}

/// Longest `AgentMessage.text` published (characters).
pub const MAX_AGENT_MESSAGE_CHARS: usize = 4_000;

/// One internet decision made by the host proxy (docs/EGRESS.md). Host,
/// verdict, reason code, size and time only: never a path, query string,
/// header or body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressDecision {
    pub task_id: TaskId,
    pub host: String,
    pub allowed: bool,
    /// Stable reason code: `allowed` or a deny code (`threat_malware`,
    /// `inspect_download_not_safe`, ...). The UI words it.
    pub reason: String,
    pub bytes: u64,
    /// Decisions dropped before this one because the UI was slow.
    pub dropped_before: u64,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TaskCreated {
        task_id: TaskId,
        title: String,
        at: DateTime<Utc>,
    },
    TaskStatusChanged {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
        at: DateTime<Utc>,
    },
    /// Agent narration for a task: model progress notes, the final
    /// summary, or why the run stopped. Model text is untrusted data:
    /// UIs render it as plain text, never markup, never instructions.
    AgentMessage {
        task_id: TaskId,
        kind: AgentMessageKind,
        text: String,
        at: DateTime<Utc>,
    },
    ComputerCreated {
        computer_id: ComputerId,
        at: DateTime<Utc>,
    },
    ComputerStateChanged {
        computer_id: ComputerId,
        from: ComputerState,
        to: ComputerState,
        at: DateTime<Utc>,
    },
    ActionRequested {
        request: ActionRequest,
    },
    ActionEvaluated {
        request: ActionRequest,
        verdict: PolicyVerdict,
    },
    ActionCompleted {
        request: ActionRequest,
        result: ActionResult,
    },
    ActionDenied {
        request: ActionRequest,
        reason: String,
    },
    /// Policy passed and dispatch began. The request is attached REDACTED
    /// (secret typing text cleared) so cursor/UI consumers can map the
    /// action to pointer motion without ever seeing secrets.
    ActionStarted {
        action_id: ActionId,
        request: ActionRequest,
        at: DateTime<Utc>,
    },
    /// Execution fault: transport error, timeout, backend failure, or
    /// interruption (takeover, pause, stop, shutdown). Terminal.
    ActionFailed {
        action_id: ActionId,
        request: ActionRequest,
        error: String,
        at: DateTime<Utc>,
    },
    /// A guest frame was captured (on demand or via `observe_after`).
    /// Pixels travel out-of-band; this event carries metadata only.
    FrameObserved {
        computer_id: ComputerId,
        frame: ObservedFrameMeta,
        action_id: Option<ActionId>,
        at: DateTime<Utc>,
    },
    /// Agent input availability changed (guest capability handshake,
    /// disconnect, backend switch). The UI disables agent affordances
    /// while unavailable — never pretends.
    InputCapabilityChanged {
        computer_id: ComputerId,
        available: bool,
        reason: String,
        at: DateTime<Utc>,
    },
    ApprovalRequested {
        task_id: TaskId,
        reason: String,
        at: DateTime<Utc>,
    },
    GuestRuntimeWaiting {
        computer_id: ComputerId,
        at: DateTime<Utc>,
    },
    GuestRuntimeConnected {
        computer_id: ComputerId,
        at: DateTime<Utc>,
    },
    GuestHandshakeCompleted {
        computer_id: ComputerId,
        protocol_version: u32,
        at: DateTime<Utc>,
    },
    GuestRuntimeReady {
        computer_id: ComputerId,
        ready_in_ms: u64,
        at: DateTime<Utc>,
    },
    GuestRuntimeDisconnected {
        computer_id: ComputerId,
        reason: String,
        at: DateTime<Utc>,
    },
    GuestRuntimeIncompatible {
        computer_id: ComputerId,
        guest_version: u32,
        at: DateTime<Utc>,
    },
    GuestRuntimeError {
        computer_id: ComputerId,
        message: String,
        at: DateTime<Utc>,
    },
    // --- Phase 4: graphical session, display, control ownership. ---
    // Emitted from REAL backend/guest facts only; never on a timer.
    /// Guest reported its compositor accepting clients (real round-trip
    /// inside the guest, not a sleep).
    GraphicalSessionReady {
        computer_id: ComputerId,
        compositor: String,
        width_px: u32,
        height_px: u32,
        /// Milliseconds from VM start to this report.
        ready_in_ms: u64,
        at: DateTime<Utc>,
    },
    /// Guest reported its compositor failed or exited.
    GraphicalSessionFailed {
        computer_id: ComputerId,
        message: String,
        at: DateTime<Utc>,
    },
    /// Host framebuffer view attached to the Pegoles window.
    DisplayAttached {
        computer_id: ComputerId,
        at: DateTime<Utc>,
    },
    /// Graphical session ready AND framebuffer attached: the human can see
    /// the computer's screen.
    DisplayReady {
        computer_id: ComputerId,
        /// Milliseconds from VM start to display readiness.
        ready_in_ms: u64,
        at: DateTime<Utc>,
    },
    /// Host framebuffer view removed (stop, destroy, explicit detach).
    DisplayDetached {
        computer_id: ComputerId,
        reason: String,
        at: DateTime<Utc>,
    },
    /// The task's computer can reach the internet (consented, per task).
    InternetOpened {
        task_id: TaskId,
        mode: InternetMode,
        domains: Vec<String>,
        at: DateTime<Utc>,
    },
    /// The task's internet access ended (kill switch: stop, pause,
    /// takeover, task end, failure).
    InternetClosed {
        task_id: TaskId,
        reason: String,
        at: DateTime<Utc>,
    },
    /// The person asked for internet but the task runs offline (not
    /// confirmed, or the channel could not open).
    InternetUnavailable {
        task_id: TaskId,
        reason: String,
        at: DateTime<Utc>,
    },
    /// Input ownership changed (e.g. user took / returned control).
    ControlOwnershipChanged {
        computer_id: ComputerId,
        from: ControlOwner,
        to: ControlOwner,
        at: DateTime<Utc>,
    },
}

impl AgentEvent {
    pub fn now() -> DateTime<Utc> {
        Utc::now()
    }
}

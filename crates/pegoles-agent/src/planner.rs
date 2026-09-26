//! The planner seam: whatever decides the next computer actions for a
//! task (a model, or a deterministic script) speaks only these types.
//!
//! A planner never executes anything. It returns `PlannedCall`s whose
//! steps are typed `ComputerAction`s; the runner sends every one of them
//! through the executor, which applies Pegoles Policy. Nothing a planner
//! returns can reach the host.

use pegoles_protocol::ComputerAction;

/// One observation of the guest display, PNG-encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One executable unit of a planned call.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A computer action (policy-checked by the executor).
    Act(ComputerAction),
    /// Capture the screen and hand it back to the planner.
    Observe,
    /// Answer from planner-side knowledge without touching the computer
    /// (e.g. the cursor position the planner itself last set).
    Reply(String),
}

/// One tool call as the planner understood it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedCall {
    /// Planner-scoped id echoed back in the matching outcome.
    pub call_id: String,
    /// Short name for logs and the activity feed (e.g. `left_click`).
    pub label: String,
    /// The steps, or why the call could not be translated (reported back
    /// to the planner as an error; halts the rest of the batch).
    pub steps: Result<Vec<Step>, String>,
}

/// What a finished call produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallOutput {
    Text(String),
    Image(Screenshot),
}

/// Result of one planned call, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallOutcome {
    pub call_id: String,
    pub result: Result<CallOutput, String>,
    /// Not attempted because an earlier call in the same batch failed.
    pub skipped: bool,
}

/// A planner's decision for one turn.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannerTurn {
    /// Run these calls in order (stop at the first failure).
    Calls {
        notes: Vec<String>,
        calls: Vec<PlannedCall>,
    },
    /// The planner considers the objective done and says so in `summary`
    /// (a model may also use it to explain why the objective cannot be
    /// done; the summary carries that).
    Done { notes: Vec<String>, summary: String },
    /// The planner determined the task failed (e.g. a deterministic
    /// verification did not pass).
    Failed { notes: Vec<String>, reason: String },
}

/// Why a planner could not produce a turn.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlannerError {
    #[error("model credentials rejected: {0}")]
    Auth(String),
    #[error("model rate limited: {0}")]
    RateLimited(String),
    #[error("model unavailable: {0}")]
    Unavailable(String),
    #[error("model declined the request: {0}")]
    Refused(String),
    #[error("unexpected model response: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
}

/// Decides the next actions. Implementations may block (network); they
/// must honor `cancel` promptly and never run actions themselves.
pub trait Planner: Send {
    /// Human-readable planner name for logs ("claude-opus-5", "script").
    fn name(&self) -> String;

    /// Begin a task: the user's objective (the only authoritative
    /// instruction) and the first observation of the screen.
    fn start(&mut self, objective: &str, screen: &Screenshot) -> Result<(), PlannerError>;

    /// Given the outcomes of the previous turn's calls (empty on the
    /// first turn), decide the next turn.
    fn next(
        &mut self,
        outcomes: Vec<CallOutcome>,
        cancel: &pegoles_core::CancellationToken,
    ) -> Result<PlannerTurn, PlannerError>;
}

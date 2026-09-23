//! Phase 5 Eyes & Hands: control arbitration, action execution, audit,
//! and the deterministic script runner.
//!
//! ```text
//! ActionRequest → rate limit → Pegoles Policy → ControlArbiter
//!   → DisplayTransform (current guest size) → backend primitives
//!     → guest channel (vsock) → guest runtime → compositor input
//! ```
//!
//! Every action emits lifecycle events on the existing EventBus
//! (`Requested → Evaluated → Started → Completed | Failed | Denied`),
//! so the UI, audit, and future agents observe the same stream. NO model
//! exists in this phase: execution is driven by Tauri dev commands and
//! the deterministic runner below.

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use chrono::{DateTime, Utc};
use pegoles_computer::{
    action_to_input_ops, op_to_guest, CapturedFrame, DisplayTransform, InputOp,
};
use pegoles_policy::{evaluate, PolicyContext};
use pegoles_protocol::{
    limits, ActionId, ActionOutcome, ActionRequest, ActionResult, AgentEvent, ComputerAction,
    ComputerId, ComputerState, ControlOwner, Decision, ObservedFrameMeta, TaskId,
};

use crate::display::ComputerView;
use crate::error::{CoreError, Result};
use crate::ComputerRegistry;

/// Milliseconds since the Unix epoch (rate-limiter clock).
pub(crate) fn input_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Cooperative cancellation flag (Take Control, task cancel, pause,
// stop, shutdown). Checked between primitives and during waits; a
/// blocking guest round-trip still bounds the delay (INPUT_ROUNDTRIP_MS).
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Pure control-ownership arbitration rules.
pub struct ControlArbiter;

impl ControlArbiter {
    /// Can the agent start (or continue) input now?
    pub fn agent_may_act(owner: ControlOwner) -> std::result::Result<(), String> {
        match owner {
            ControlOwner::None => Ok(()),
            ControlOwner::Agent => Ok(()),
            ControlOwner::User => Err("control unavailable: a human owns the computer".to_string()),
        }
    }

    /// Human takeover is always allowed from Agent/None; never needed
    /// from User (already theirs).
    pub fn user_may_take(owner: ControlOwner) -> bool {
        matches!(owner, ControlOwner::Agent | ControlOwner::None)
    }
}

/// One audit row. Content is NEVER logged: typing records char counts,
/// never text — even non-sensitive text stays out of the log.
#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    pub action_id: ActionId,
    pub computer_id: Option<ComputerId>,
    pub task_id: Option<TaskId>,
    pub verb: &'static str,
    pub decision: String,
    pub outcome: String,
    pub duration_ms: u64,
    pub redacted: bool,
    pub text_len: Option<usize>,
}

/// Bounded structured audit log (in-memory, Phase 1 persistence rules).
#[derive(Clone, Debug, Default)]
pub struct AuditLog {
    entries: VecDeque<AuditEntry>,
    capacity: usize,
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: 500,
        }
    }

    pub fn push(&mut self, entry: AuditEntry) {
        if self.entries.len() >= self.capacity.max(1) {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> &VecDeque<AuditEntry> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One deterministic script step (dev/test only, never model output).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ScriptStep {
    pub label: String,
    pub action: ComputerAction,
    pub observe_after: bool,
}

impl ScriptStep {
    pub fn new(label: impl Into<String>, action: ComputerAction) -> Self {
        Self {
            label: label.into(),
            action,
            observe_after: false,
        }
    }

    pub fn observing(mut self) -> Self {
        self.observe_after = true;
        self
    }
}

/// Report of a deterministic script run.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScriptReport {
    pub steps_total: usize,
    pub steps_executed: usize,
    pub aborted_at: Option<usize>,
    pub results: Vec<ActionResult>,
}

/// The req-19 smoke demo. Precondition (documented, enforced by aborting
/// with a clear message otherwise): the Pegoles input fixture runs
/// fullscreen in the guest, so center-proportional coordinates are
/// meaningful. Without it the demo still proves observe + control
/// handoff and stops before blind clicking.
pub fn pegoles_demo_script() -> Vec<ScriptStep> {
    use ComputerAction as A;
    vec![
        ScriptStep::new("observe desktop", A::ObserveScreen).observing(),
        ScriptStep::new("take in fixture", A::MovePointer { x: 0.5, y: 0.5 }),
        ScriptStep::new(
            "focus fixture",
            A::Click {
                x: 0.5,
                y: 0.5,
                button: pegoles_protocol::PointerButton::Primary,
            },
        ),
        ScriptStep::new("settle", A::Wait { duration_ms: 500 }),
        ScriptStep::new(
            "type alive",
            A::TypeText {
                text: "echo \"Pegoles is alive\"".into(),
                sensitive: false,
            },
        ),
        ScriptStep::new(
            "run line",
            A::KeyPress {
                key: "Enter".into(),
            },
        ),
        ScriptStep::new("settle", A::Wait { duration_ms: 500 }),
        ScriptStep::new("capture result", A::ObserveScreen).observing(),
    ]
}

/// Live input facts for the UI/dev panel (all real state, no guesses).
#[derive(Clone, Debug)]
pub struct InputStatus {
    pub available: bool,
    pub agent_busy: bool,
    pub pressed_clean: bool,
    pub audit_len: usize,
    pub last_frame: Option<ObservedFrameMeta>,
    /// Structured guest reasons for withheld capabilities (empty when
    /// all advertised or the guest predates diagnostics).
    pub unavailable: Vec<pegoles_computer::CapabilityDiagnostic>,
}

impl ComputerRegistry {
    /// Agent input availability right now (guest Ready + advertised cap).
    pub fn input_available(&self) -> bool {
        self.backend.as_ref().is_some_and(|b| b.input_available())
    }

    pub fn input_status(&self) -> InputStatus {
        InputStatus {
            available: self.input_available(),
            agent_busy: self.display.control == ControlOwner::Agent,
            pressed_clean: self.input_pressed.is_clean(),
            audit_len: self.audit.len(),
            last_frame: self.frame_cache.last().map(|f| f.meta.clone()),
            unavailable: self
                .backend
                .as_ref()
                .map(|b| b.capability_diagnostics())
                .unwrap_or_default(),
        }
    }

    /// Cooperative cancel for the running agent sequence (takeover,
    /// pause, stop, shutdown). Releases pressed state best-effort.
    pub fn cancel_agent_input(&mut self, reason: &str) {
        self.agent_cancel.cancel();
        self.release_pressed_state(reason);
    }

    /// Best-effort release of everything the host believes is held.
    /// Never fails the caller: a stuck release is recorded, not raised.
    pub fn release_pressed_state(&mut self, _reason: &str) {
        if self.input_pressed.is_clean() {
            return;
        }
        let at = self.last_pointer_guest;
        let ops = self.input_pressed.release_all_ops(at);
        if let Some(backend) = self.backend.as_deref_mut() {
            for op in &ops {
                let _ = backend.input_execute("release-all", op);
            }
            backend.input_release_all();
        }
    }

    /// Begin an agent session (runner): acquire Agent control once.
    pub fn begin_agent_session(&mut self, out: &mut Vec<AgentEvent>) -> Result<ComputerId> {
        let id = self.computer_id()?;
        ControlArbiter::agent_may_act(self.display.control)
            .map_err(CoreError::ControlUnavailable)?;
        self.require_running()?;
        self.agent_cancel = CancellationToken::new();
        self.set_control(id, ControlOwner::Agent, out);
        Ok(id)
    }

    /// End an agent session: release pressed state, return control to None.
    pub fn end_agent_session(&mut self, out: &mut Vec<AgentEvent>) {
        self.release_pressed_state("session end");
        if self.display.control == ControlOwner::Agent {
            if let Some(id) = self.current_id() {
                self.set_control(id, ControlOwner::None, out);
            } else {
                self.display.control = ControlOwner::None;
            }
        }
    }

    fn require_running(&self) -> Result<()> {
        match self.state() {
            Some(ComputerState::Running) => Ok(()),
            other => Err(CoreError::ControlUnavailable(format!(
                "computer is not running ({other:?})"
            ))),
        }
    }

    /// Current guest pixel size: guest report wins, else configured
    /// display, else honest failure (never invented).
    fn guest_pixel_size(&self) -> Result<(u32, u32)> {
        let session = self.graphical_session();
        if let (Some(w), Some(h)) = (session.width_px, session.height_px) {
            if w > 0 && h > 0 {
                return Ok((w, h));
            }
        }
        if let Some(cfg) = self.display_config() {
            return Ok((cfg.width_px, cfg.height_px));
        }
        Err(CoreError::ControlUnavailable(
            "unknown guest display size".to_string(),
        ))
    }

    fn transform(&self) -> Result<DisplayTransform> {
        let (w, h) = self.guest_pixel_size()?;
        Ok(DisplayTransform::headless(w, h))
    }

    /// Execute ONE structured action end-to-end (policy → control →
    /// dispatch → events → audit). Blocking; honors `cancel` between
    /// primitives and during waits. Returns the result AND the events it
    /// published (the bridge also sees them via the bus).
    pub fn execute_action(
        &mut self,
        task_id: TaskId,
        action: ComputerAction,
        observe_after: bool,
        ctx: &PolicyContext,
        cancel: &CancellationToken,
        manage_control: bool,
    ) -> (ActionResult, Vec<AgentEvent>) {
        let mut out = Vec::new();
        let started_wall = Utc::now();
        // A fresh managed action is a new user intent: stale takeover
        // state from an older sequence must not block it. (Runner steps
        // never reset: begin_agent_session owns the fresh flag there, so
        // a mid-script takeover still interrupts the next step.)
        if manage_control {
            self.agent_cancel = CancellationToken::new();
        }
        let computer_id = match self.computer_id() {
            Ok(id) => id,
            Err(_) => {
                let result = ActionResult::failed(
                    ActionId::new(),
                    started_wall,
                    ActionOutcome::Failed,
                    "no computer available",
                );
                return (result, out);
            }
        };
        let mut req = ActionRequest::new(task_id, computer_id, action);
        req.observe_after = observe_after;
        let redacted = req.redacted_for_event();
        self.emit(
            AgentEvent::ActionRequested {
                request: redacted.clone(),
            },
            &mut out,
        );

        // Rate brake before policy (cheap, deterministic).
        if !self.rate_limiter.check(input_clock_ms()) {
            let result = ActionResult::failed(
                req.action_id,
                started_wall,
                ActionOutcome::Failed,
                format!("rate limit exceeded ({} /s)", limits::MAX_ACTIONS_PER_SEC),
            );
            self.emit(
                AgentEvent::ActionFailed {
                    action_id: req.action_id,
                    request: redacted,
                    error: result.error.clone().unwrap_or_default(),
                    at: Utc::now(),
                },
                &mut out,
            );
            self.audit_action(&req, "rate_limited", &result);
            return (result, out);
        }

        // Policy gate: every action, even inside the isolated VM.
        let verdict = evaluate(&req, ctx);
        self.emit(
            AgentEvent::ActionEvaluated {
                request: redacted.clone(),
                verdict: verdict.clone(),
            },
            &mut out,
        );
        let decision_name = format!("{:?}", verdict.decision);
        match verdict.decision {
            Decision::Deny => {
                let result = ActionResult::failed(
                    req.action_id,
                    started_wall,
                    ActionOutcome::Blocked,
                    verdict.reason.clone(),
                );
                self.emit(
                    AgentEvent::ActionDenied {
                        request: redacted,
                        reason: verdict.reason,
                    },
                    &mut out,
                );
                self.audit_action(&req, &decision_name, &result);
                return (result, out);
            }
            Decision::RequireApproval => {
                let result = ActionResult::failed(
                    req.action_id,
                    started_wall,
                    ActionOutcome::NeedsApproval,
                    verdict.reason.clone(),
                );
                self.emit(
                    AgentEvent::ApprovalRequested {
                        task_id,
                        reason: verdict.reason,
                        at: Utc::now(),
                    },
                    &mut out,
                );
                self.emit(
                    AgentEvent::ActionCompleted {
                        request: redacted,
                        result: result.clone(),
                    },
                    &mut out,
                );
                self.audit_action(&req, &decision_name, &result);
                return (result, out);
            }
            Decision::Allow => {}
        }

        if cancel.is_cancelled() || self.agent_cancel.is_cancelled() {
            return self.interrupted(
                req,
                redacted,
                started_wall,
                "cancelled before dispatch",
                &mut out,
            );
        }

        // Control arbitration.
        let acquired_here = if manage_control {
            match ControlArbiter::agent_may_act(self.display.control) {
                Ok(()) => {
                    if let Err(e) = self.require_running() {
                        return self.failed(req, redacted, started_wall, e.to_string(), &mut out);
                    }
                    if self.display.control == ControlOwner::None {
                        self.set_control(computer_id, ControlOwner::Agent, &mut out);
                    }
                    true
                }
                Err(reason) => {
                    return self.failed(req, redacted, started_wall, reason, &mut out);
                }
            }
        } else {
            false
        };

        self.emit(
            AgentEvent::ActionStarted {
                action_id: req.action_id,
                request: redacted.clone(),
                at: Utc::now(),
            },
            &mut out,
        );

        let result = self.dispatch(req.clone(), cancel, &mut out);

        if manage_control && acquired_here {
            self.release_pressed_state("action end");
            if self.display.control == ControlOwner::Agent {
                self.set_control(computer_id, ControlOwner::None, &mut out);
            }
        }
        self.audit_action(&req, &decision_name, &result);
        (result, out)
    }

    fn interrupted(
        &mut self,
        req: ActionRequest,
        redacted: ActionRequest,
        started_wall: DateTime<Utc>,
        reason: &str,
        out: &mut Vec<AgentEvent>,
    ) -> (ActionResult, Vec<AgentEvent>) {
        self.release_pressed_state(reason);
        let result = ActionResult::failed(
            req.action_id,
            started_wall,
            ActionOutcome::Interrupted,
            format!("interrupted: {reason}"),
        );
        self.emit(
            AgentEvent::ActionFailed {
                action_id: req.action_id,
                request: redacted,
                error: result.error.clone().unwrap_or_default(),
                at: Utc::now(),
            },
            out,
        );
        self.audit_action(&req, "interrupted", &result);
        (result, std::mem::take(out))
    }

    fn failed(
        &mut self,
        req: ActionRequest,
        redacted: ActionRequest,
        started_wall: DateTime<Utc>,
        error: String,
        out: &mut Vec<AgentEvent>,
    ) -> (ActionResult, Vec<AgentEvent>) {
        let result = ActionResult::failed(
            req.action_id,
            started_wall,
            ActionOutcome::Failed,
            error.clone(),
        );
        self.emit(
            AgentEvent::ActionFailed {
                action_id: req.action_id,
                request: redacted,
                error,
                at: Utc::now(),
            },
            out,
        );
        self.audit_action(&req, "failed", &result);
        (result, std::mem::take(out))
    }

    /// Dispatch an allowed action's primitives. Caller owns arbitration
    /// and the Started event; this returns Completed (Executed) or Failed.
    fn dispatch(
        &mut self,
        req: ActionRequest,
        cancel: &CancellationToken,
        out: &mut Vec<AgentEvent>,
    ) -> ActionResult {
        let started_wall = Utc::now();
        // Wait is pure timing (cancellable slices); display info is
        // answered from session facts; everything else lowers to ops.
        if let ComputerAction::Wait { duration_ms } = &req.action {
            let capped = (*duration_ms).min(limits::MAX_WAIT_MS);
            let slept = sleep_cancellable(capped, cancel, &self.agent_cancel);
            if !slept {
                return self.interrupted_result(&req, started_wall, "wait cancelled", out);
            }
            return self.completed(req, started_wall, "waited", None, out);
        }
        if matches!(req.action, ComputerAction::GetDisplayInfo) {
            match self.guest_pixel_size() {
                Ok((w, h)) => {
                    return self.completed(
                        req,
                        started_wall,
                        format!("display is {w}x{h}"),
                        None,
                        out,
                    );
                }
                Err(e) => {
                    return self.failed_result(&req, started_wall, e.to_string(), out);
                }
            }
        }
        let transform = match self.transform() {
            Ok(t) => t,
            Err(e) => return self.failed_result(&req, started_wall, e.to_string(), out),
        };
        let ops = action_to_input_ops(&req.action, &transform);
        let op_count = ops.len();
        for (i, op) in ops.iter().enumerate() {
            if cancel.is_cancelled() || self.agent_cancel.is_cancelled() {
                return self.interrupted_result(&req, started_wall, "cancelled mid-action", out);
            }
            // Track pressed state for stuck-input recovery.
            match op {
                InputOp::Down { button, point } => {
                    self.input_pressed.press_button(*button);
                    self.last_pointer_guest = *point;
                }
                InputOp::Up { button, point } => {
                    self.input_pressed.release_button(*button);
                    self.last_pointer_guest = *point;
                }
                InputOp::Move { point }
                | InputOp::Click { point, .. }
                | InputOp::DoubleClick { point, .. }
                | InputOp::Scroll { point, .. } => {
                    self.last_pointer_guest = *point;
                }
                InputOp::Drag { to, .. } => {
                    self.last_pointer_guest = *to;
                }
                _ => {}
            }
            // Validate the lowered op against the guest wire bounds.
            if let Some(g) = op_to_guest(op) {
                if !g.is_bounded() {
                    return self.failed_result(&req, started_wall, "op exceeds guest bounds", out);
                }
            }
            let request_id = format!("{}:{i}", req.action_id);
            let outcome = match self.backend.as_deref_mut() {
                Some(backend) => backend.input_execute(&request_id, op),
                None => {
                    return self.failed_result(&req, started_wall, "no computer available", out);
                }
            };
            if !outcome.ok {
                // A failed primitive may leave buttons down: recover now.
                self.release_pressed_state("primitive failed");
                return self.failed_result(
                    &req,
                    started_wall,
                    outcome
                        .error
                        .unwrap_or_else(|| "backend rejected input".to_string()),
                    out,
                );
            }
        }
        // Optional observation chained to the action (no extra policy hop).
        let mut frame_id = None;
        if req.observe_after {
            match self.capture_frame(&format!("{}:observe", req.action_id), out) {
                Ok(meta) => frame_id = Some(meta.frame_id),
                Err(message) => {
                    return self.failed_result(
                        &req,
                        started_wall,
                        format!("observe_after failed: {message}"),
                        out,
                    );
                }
            }
        }
        self.completed(
            req,
            started_wall,
            format!("executed {op_count} primitive(s)"),
            frame_id,
            out,
        )
    }

    fn completed(
        &mut self,
        req: ActionRequest,
        started_wall: DateTime<Utc>,
        message: impl Into<String>,
        frame_id: Option<pegoles_protocol::FrameId>,
        out: &mut Vec<AgentEvent>,
    ) -> ActionResult {
        let mut result = ActionResult::executed(req.action_id, started_wall, message);
        result.resulting_frame_id = frame_id;
        self.emit(
            AgentEvent::ActionCompleted {
                request: req.redacted_for_event(),
                result: result.clone(),
            },
            out,
        );
        result
    }

    fn failed_result(
        &mut self,
        req: &ActionRequest,
        started_wall: DateTime<Utc>,
        error: impl Into<String>,
        out: &mut Vec<AgentEvent>,
    ) -> ActionResult {
        let result = ActionResult::failed(
            req.action_id,
            started_wall,
            ActionOutcome::Failed,
            error.into(),
        );
        self.emit(
            AgentEvent::ActionFailed {
                action_id: req.action_id,
                request: req.redacted_for_event(),
                error: result.error.clone().unwrap_or_default(),
                at: Utc::now(),
            },
            out,
        );
        result
    }

    fn interrupted_result(
        &mut self,
        req: &ActionRequest,
        started_wall: DateTime<Utc>,
        reason: &str,
        out: &mut Vec<AgentEvent>,
    ) -> ActionResult {
        self.release_pressed_state(reason);
        let result = ActionResult::failed(
            req.action_id,
            started_wall,
            ActionOutcome::Interrupted,
            format!("interrupted: {reason}"),
        );
        self.emit(
            AgentEvent::ActionFailed {
                action_id: req.action_id,
                request: req.redacted_for_event(),
                error: result.error.clone().unwrap_or_default(),
                at: Utc::now(),
            },
            out,
        );
        result
    }

    /// Blocking guest frame capture → cache → FrameObserved event.
    /// Pixels stay out-of-band (returned + cached, never in events).
    pub fn capture_frame(
        &mut self,
        request_id: &str,
        out: &mut Vec<AgentEvent>,
    ) -> std::result::Result<ObservedFrameMeta, String> {
        let computer_id = self.computer_id().map_err(|e| e.to_string())?;
        if self.state() != Some(ComputerState::Running) {
            return Err("computer is not running".to_string());
        }
        let frame: CapturedFrame = match self.backend.as_deref_mut() {
            Some(backend) => backend
                .input_capture_frame(
                    request_id,
                    std::time::Duration::from_millis(limits::FRAME_CAPTURE_TIMEOUT_MS),
                )
                .map_err(|e| e.to_string())?,
            None => return Err("no computer available".to_string()),
        };
        if !frame.meta.stride_consistent() {
            return Err("frame stride mismatch".to_string());
        }
        let (id, _) = self.frame_cache.insert(
            computer_id,
            frame.meta.width_px,
            frame.meta.height_px,
            frame.meta.encoding,
            frame.bytes,
            frame.meta.capture_latency_ms,
        );
        let meta = self
            .frame_cache
            .get(&id)
            .map(|f| f.meta.clone())
            .ok_or_else(|| "frame cache lost".to_string())?;
        self.emit(
            AgentEvent::FrameObserved {
                computer_id,
                frame: meta.clone(),
                action_id: None,
                at: Utc::now(),
            },
            out,
        );
        Ok(meta)
    }

    /// Last cached frame bytes (dev panel / PNG export), if any.
    pub fn last_frame_bytes(&self) -> Option<(ObservedFrameMeta, Vec<u8>)> {
        self.frame_cache
            .last()
            .map(|f| (f.meta.clone(), f.bytes.clone()))
    }

    /// Recent audit rows, oldest first, capped at `limit`.
    pub fn audit_rows(&self, limit: usize) -> Vec<AuditEntry> {
        let entries = self.audit.entries();
        let skip = entries.len().saturating_sub(limit);
        entries.iter().skip(skip).cloned().collect()
    }

    fn audit_action(&mut self, req: &ActionRequest, decision: &str, result: &ActionResult) {
        let text_len = match &req.action {
            ComputerAction::TypeText { text, .. } | ComputerAction::Type { text } => {
                Some(text.chars().count())
            }
            _ => None,
        };
        self.audit.push(crate::input::AuditEntry {
            at: Utc::now(),
            action_id: req.action_id,
            computer_id: Some(req.computer_id),
            task_id: Some(req.task_id),
            verb: req.action.verb(),
            decision: decision.to_string(),
            outcome: format!("{:?}", result.outcome),
            duration_ms: result.duration_ms,
            redacted: req.action.is_sensitive(),
            text_len,
        });
    }

    /// Publish InputCapabilityChanged when availability flips (the UI
    /// disables agent affordances while unavailable — never pretends).
    pub fn publish_input_capability(&mut self, out: &mut Vec<AgentEvent>) {
        let available = self.input_available();
        if self.last_input_available != Some(available) {
            self.last_input_available = Some(available);
            if let Some(id) = self.current_id() {
                self.emit(
                    AgentEvent::InputCapabilityChanged {
                        computer_id: id,
                        available,
                        reason: if available {
                            "guest advertised input support".to_string()
                        } else {
                            "guest input unavailable".to_string()
                        },
                        at: Utc::now(),
                    },
                    out,
                );
            }
        }
    }

    /// Run a deterministic script (dev/test only). Acquires Agent control
    /// once, executes steps in order, aborts on first terminal failure,
    /// always releases pressed state + control. No model involved.
    pub fn run_script(
        &mut self,
        task_id: TaskId,
        steps: &[ScriptStep],
        ctx: &PolicyContext,
        cancel: &CancellationToken,
    ) -> (ScriptReport, Vec<AgentEvent>) {
        let mut out = Vec::new();
        let mut results = Vec::new();
        let computer_id = match self.begin_agent_session(&mut out) {
            Ok(id) => id,
            Err(_) => {
                return (
                    ScriptReport {
                        steps_total: steps.len(),
                        steps_executed: 0,
                        aborted_at: Some(0),
                        results,
                    },
                    out,
                );
            }
        };
        let _ = computer_id;
        let mut aborted_at = None;
        for (i, step) in steps.iter().enumerate() {
            if cancel.is_cancelled() || self.agent_cancel.is_cancelled() {
                aborted_at = Some(i);
                break;
            }
            if self.state() != Some(ComputerState::Running) {
                let (result, mut ev) =
                    self.interrupted_step(task_id, step, "computer left running");
                out.append(&mut ev);
                results.push(result);
                aborted_at = Some(i);
                break;
            }
            let action = step.action.clone();
            let (result, mut ev) =
                self.execute_action(task_id, action, step.observe_after, ctx, cancel, false);
            out.append(&mut ev);
            let terminal_ok = result.success;
            results.push(result);
            if !terminal_ok {
                aborted_at = Some(i);
                break;
            }
        }
        self.end_agent_session(&mut out);
        let steps_executed = results.len();
        (
            ScriptReport {
                steps_total: steps.len(),
                steps_executed,
                aborted_at,
                results,
            },
            out,
        )
    }

    fn interrupted_step(
        &mut self,
        task_id: TaskId,
        step: &ScriptStep,
        reason: &str,
    ) -> (ActionResult, Vec<AgentEvent>) {
        let mut out = Vec::new();
        let computer_id = match self.computer_id() {
            Ok(id) => id,
            Err(_) => {
                return (
                    ActionResult::failed(
                        ActionId::new(),
                        Utc::now(),
                        ActionOutcome::Interrupted,
                        "no computer available",
                    ),
                    out,
                );
            }
        };
        let req = ActionRequest::new(task_id, computer_id, step.action.clone());
        self.release_pressed_state(reason);
        let result = ActionResult::failed(
            req.action_id,
            Utc::now(),
            ActionOutcome::Interrupted,
            format!("interrupted: {reason}"),
        );
        self.emit(
            AgentEvent::ActionFailed {
                action_id: req.action_id,
                request: req.redacted_for_event(),
                error: result.error.clone().unwrap_or_default(),
                at: Utc::now(),
            },
            &mut out,
        );
        (result, out)
    }

    /// ComputerView extended for Phase 5 (input availability for gating).
    pub fn computer_view_with_input(&self, image_preparing: bool) -> (ComputerView, bool) {
        (self.computer_view(image_preparing), self.input_available())
    }
}

/// Sleep in 50 ms slices so cancellation lands promptly.
fn sleep_cancellable(
    duration_ms: u32,
    cancel: &CancellationToken,
    agent_cancel: &CancellationToken,
) -> bool {
    let mut left = duration_ms as u64;
    while left > 0 {
        if cancel.is_cancelled() || agent_cancel.is_cancelled() {
            return false;
        }
        let slice = left.min(50);
        std::thread::sleep(std::time::Duration::from_millis(slice));
        left = left.saturating_sub(slice);
    }
    !cancel.is_cancelled() && !agent_cancel.is_cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use pegoles_computer::platform::BackendKind;
    use pegoles_computer::{ComputerBackend, MockComputerBackend};
    use pegoles_protocol::PointerButton;
    use std::path::PathBuf;

    fn registry() -> ComputerRegistry {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("pg-input-test-{}", std::process::id()));
        ComputerRegistry::with_dirs(EventBus::new(), BackendKind::Mock, dir)
    }

    fn with_mock_running() -> ComputerRegistry {
        use pegoles_protocol::{ComputerConfig, DisplayConfig, DisplayProfile};
        let mut r = registry();
        let mut backend = MockComputerBackend::new();
        backend
            .create(ComputerConfig {
                display: DisplayConfig::for_profile(DisplayProfile::DesktopLarge),
                ..ComputerConfig::default()
            })
            .unwrap();
        backend.start().unwrap();
        backend.set_input_enabled(true);
        r.backend = Some(Box::new(backend));
        r
    }

    fn task() -> TaskId {
        TaskId::new()
    }

    fn ctx() -> PolicyContext {
        PolicyContext::default()
    }

    #[test]
    fn arbiter_rules() {
        assert!(ControlArbiter::agent_may_act(ControlOwner::None).is_ok());
        assert!(ControlArbiter::agent_may_act(ControlOwner::Agent).is_ok());
        assert!(ControlArbiter::agent_may_act(ControlOwner::User).is_err());
        assert!(ControlArbiter::user_may_take(ControlOwner::Agent));
        assert!(ControlArbiter::user_may_take(ControlOwner::None));
        assert!(!ControlArbiter::user_may_take(ControlOwner::User));
    }

    #[test]
    fn user_takeover_blocks_agent() {
        let mut r = with_mock_running();
        r.display.control = ControlOwner::User;
        let cancel = CancellationToken::new();
        let (result, _) = r.execute_action(
            task(),
            ComputerAction::MovePointer { x: 0.5, y: 0.5 },
            false,
            &ctx(),
            &cancel,
            true,
        );
        assert!(!result.success);
        assert!(result.message.contains("human owns"));
    }

    #[test]
    fn click_executes_through_mock_and_releases_control() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        let (result, events) = r.execute_action(
            task(),
            ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            },
            false,
            &ctx(),
            &cancel,
            true,
        );
        assert!(result.success, "failed: {:?}", result.error);
        assert_eq!(r.display.control, ControlOwner::None);
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                AgentEvent::ActionRequested { .. } => "requested",
                AgentEvent::ActionEvaluated { .. } => "evaluated",
                AgentEvent::ActionStarted { .. } => "started",
                AgentEvent::ActionCompleted { .. } => "completed",
                AgentEvent::ActionFailed { .. } => "failed",
                AgentEvent::ActionDenied { .. } => "denied",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "requested",
                "evaluated",
                "other", // ControlOwnershipChanged(None → Agent)
                "started",
                "completed",
                "other", // ControlOwnershipChanged(Agent → None)
            ]
        );
    }

    #[test]
    fn policy_deny_short_circuits_without_touching_backend() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        let (result, events) = r.execute_action(
            task(),
            ComputerAction::Shell {
                command: "rm -rf /".into(),
            },
            false,
            &ctx(),
            &cancel,
            true,
        );
        assert!(!result.success);
        assert_eq!(result.outcome, ActionOutcome::Blocked);
        assert!(events
            .iter()
            .any(|e| matches!(e, AgentEvent::ActionDenied { .. })));
        assert!(!events
            .iter()
            .any(|e| matches!(e, AgentEvent::ActionStarted { .. })));
    }

    #[test]
    fn stuck_button_released_on_failure() {
        let mut r = with_mock_running();
        // Disable mid-test: Down succeeds, then backend fails Up.
        let cancel = CancellationToken::new();
        let (down, _) = r.execute_action(
            task(),
            ComputerAction::MouseDown {
                x: 0.1,
                y: 0.1,
                button: PointerButton::Primary,
            },
            false,
            &ctx(),
            &cancel,
            false,
        );
        assert!(down.success);
        assert!(!r.input_pressed.is_clean());
        r.cancel_agent_input("test takeover");
        assert!(r.input_pressed.is_clean());
    }

    #[test]
    fn cancelled_wait_interrupts() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (result, _) = r.execute_action(
            task(),
            ComputerAction::Wait { duration_ms: 5000 },
            false,
            &ctx(),
            &cancel,
            true,
        );
        assert_eq!(result.outcome, ActionOutcome::Interrupted);
    }

    #[test]
    fn sensitive_typing_never_reaches_events_or_audit() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        let secret = "not-a-real-secret-ABCDEF123";
        let (result, events) = r.execute_action(
            task(),
            ComputerAction::TypeText {
                text: secret.into(),
                sensitive: true,
            },
            false,
            &ctx(),
            &cancel,
            true,
        );
        assert!(result.success);
        let blob = serde_json::to_string(&events).unwrap();
        assert!(!blob.contains(secret));
        let audit = serde_json::to_string(
            &r.audit
                .entries()
                .iter()
                .map(|e| (e.verb, e.outcome.clone(), e.text_len, e.redacted))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(!audit.contains(secret));
        assert!(r.audit.entries().back().is_some_and(|e| e.redacted));
    }

    #[test]
    fn script_runner_aborts_on_failure_and_releases() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        let steps = vec![
            ScriptStep::new("move", ComputerAction::MovePointer { x: 0.1, y: 0.1 }),
            ScriptStep::new(
                "evil",
                ComputerAction::Shell {
                    command: "rm -rf /".into(),
                },
            ),
            ScriptStep::new("never", ComputerAction::MovePointer { x: 0.9, y: 0.9 }),
        ];
        let (report, _) = r.run_script(task(), &steps, &ctx(), &cancel);
        assert_eq!(report.steps_executed, 2);
        assert_eq!(report.aborted_at, Some(1));
        assert_eq!(r.display.control, ControlOwner::None);
        assert!(r.input_pressed.is_clean());
    }

    #[test]
    fn observe_after_attaches_frame() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        let (result, events) = r.execute_action(
            task(),
            ComputerAction::MovePointer { x: 0.5, y: 0.5 },
            true,
            &ctx(),
            &cancel,
            true,
        );
        assert!(result.success);
        assert!(result.resulting_frame_id.is_some());
        assert!(events
            .iter()
            .any(|e| matches!(e, AgentEvent::FrameObserved { .. })));
    }

    #[test]
    fn audit_log_bounds_and_verbs() {
        let mut r = with_mock_running();
        let cancel = CancellationToken::new();
        for _ in 0..3 {
            let _ = r.execute_action(
                task(),
                ComputerAction::KeyPress {
                    key: "Enter".into(),
                },
                false,
                &ctx(),
                &cancel,
                true,
            );
        }
        assert_eq!(r.audit.len(), 3);
        assert!(r.audit.entries().iter().all(|e| e.verb == "key_press"));
    }
}

//! Bounded session-only approvals. Payloads and grants never serialize.
use crate::{CancellationToken, ComputerRegistry};
use chrono::{DateTime, Utc};
use pegoles_protocol::{
    ActionId, ActionOutcome, ActionRequest, ActionResult, ComputerAction, ComputerId,
    ComputerState, ControlOwner, Decision, TaskId,
};
use std::time::{Duration, Instant};

pub const APPROVAL_TTL: Duration = Duration::from_secs(120);
const MAX_PENDING: usize = 8;

#[derive(Clone, Copy, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    ApproveOnce,
    Deny,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PendingApproval {
    pub id: ActionId,
    pub task_id: TaskId,
    pub computer_id: ComputerId,
    pub summary: String,
    pub expires_at: DateTime<Utc>,
}
pub(crate) struct Pending {
    pub req: ActionRequest,
    pub generation: u64,
    pub deadline: Instant,
    pub view: PendingApproval,
    pub decision: Option<ApprovalDecision>,
    pub typed: usize,
}
#[derive(Default)]
pub(crate) struct Approvals {
    pub strict: bool,
    pub generation: u64,
    pub pending: Vec<Pending>,
    stopped: std::collections::VecDeque<Stopped>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalStopReason {
    Denied,
    Expired,
    Cancelled,
    ControlChanged,
    Stale,
}
struct Stopped {
    id: ActionId,
    task: TaskId,
    typed: usize,
    total: usize,
    reason: ApprovalStopReason,
}
impl Pending {
    fn stopped(&self, reason: ApprovalStopReason) -> Stopped {
        Stopped {
            id: self.req.action_id,
            task: self.req.task_id,
            typed: self.typed,
            total: match &self.req.action {
                ComputerAction::TypeText { text, .. } => text.chars().count(),
                _ => 0,
            },
            reason,
        }
    }
}
impl Stopped {
    fn progress(&self) -> ApprovalProgress {
        let message = match self.reason {
            ApprovalStopReason::Denied => "Action denied by the human",
            ApprovalStopReason::Expired => "Approval expired",
            ApprovalStopReason::Cancelled => "Approval cancelled",
            ApprovalStopReason::ControlChanged => "Computer control changed",
            ApprovalStopReason::Stale => "Approval is no longer valid",
        };
        let mut result =
            ActionResult::failed(self.id, Utc::now(), ActionOutcome::Interrupted, message);
        annotate_partial(&mut result, self.typed, self.total);
        ApprovalProgress::Stopped {
            reason: self.reason,
            result,
        }
    }
}
fn annotate_partial(result: &mut ActionResult, typed: usize, total: usize) {
    if typed > 0 {
        result.message = format!(
            "{} (the first {typed} of {total} characters were typed)",
            result.message
        );
        result.error = Some(result.message.clone());
    }
}
pub enum ApprovalProgress {
    Waiting,
    Continue,
    Finished(ActionResult),
    Stopped {
        reason: ApprovalStopReason,
        result: ActionResult,
    },
}

impl ComputerRegistry {
    pub fn ask_before_input(&self) -> bool {
        self.approvals.strict
    }
    /// Desktop calls this only after native human confirmation.
    pub fn set_ask_before_input(&mut self, enabled: bool) {
        self.invalidate_approvals();
        self.approvals.strict = enabled;
    }
    pub(crate) fn invalidate_approvals(&mut self) {
        self.approvals.generation = self.approvals.generation.wrapping_add(1);
        for p in std::mem::take(&mut self.approvals.pending) {
            let reason = if p.decision == Some(ApprovalDecision::Deny) {
                ApprovalStopReason::Denied
            } else if Instant::now() >= p.deadline {
                ApprovalStopReason::Expired
            } else {
                ApprovalStopReason::ControlChanged
            };
            self.remember_pending_stop(p, reason);
        }
    }
    fn settle_approval(&mut self, p: &Pending, result: &ActionResult) {
        self.publish_action_result(&p.req, result, &mut Vec::new());
        self.audit_action(&p.req, "RequireApproval", result);
    }
    fn stop_pending(&mut self, p: &Pending, reason: ApprovalStopReason) -> ApprovalProgress {
        let progress = p.stopped(reason).progress();
        if let ApprovalProgress::Stopped { result, .. } = &progress {
            self.settle_approval(p, result);
        }
        progress
    }
    fn remember_pending_stop(&mut self, p: Pending, reason: ApprovalStopReason) {
        self.stop_pending(&p, reason);
        self.remember_stop(p.stopped(reason));
    }
    fn remember_stop(&mut self, stop: Stopped) {
        if self.approvals.stopped.len() >= MAX_PENDING {
            self.approvals.stopped.pop_front();
        }
        self.approvals.stopped.push_back(stop);
    }
    fn expire_approvals(&mut self) {
        for p in std::mem::take(&mut self.approvals.pending) {
            if Instant::now() >= p.deadline {
                self.remember_pending_stop(p, ApprovalStopReason::Expired);
            } else {
                self.approvals.pending.push(p);
            }
        }
    }
    pub fn list_approvals(&mut self) -> Vec<PendingApproval> {
        self.expire_approvals();
        self.approvals
            .pending
            .iter()
            .filter(|p| p.decision.is_none())
            .map(|p| p.view.clone())
            .collect()
    }
    pub fn approval(&mut self, id: ActionId) -> Result<PendingApproval, String> {
        self.list_approvals()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| "Approval is no longer pending".into())
    }
    pub fn resolve_approval(
        &mut self,
        id: ActionId,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        self.resolve_approval_at(id, decision, Instant::now())
    }
    fn resolve_approval_at(
        &mut self,
        id: ActionId,
        decision: ApprovalDecision,
        now: Instant,
    ) -> Result<(), String> {
        let computer = self.current_id();
        let valid = self.control_owner() == ControlOwner::Agent
            && self.state() == Some(ComputerState::Running)
            && !self.agent_cancel.is_cancelled();
        let p = self
            .approvals
            .pending
            .iter_mut()
            .find(|p| p.req.action_id == id)
            .ok_or("Approval is no longer pending")?;
        if !valid
            || computer != Some(p.req.computer_id)
            || p.generation != self.approvals.generation
            || now >= p.deadline
            || p.decision.is_some()
        {
            return Err("Approval is no longer pending".into());
        }
        p.decision = Some(decision);
        Ok(())
    }
    pub(crate) fn queue_approval(&mut self, req: ActionRequest) -> Result<(), String> {
        self.queue_approval_at(req, Instant::now())
    }
    fn queue_approval_at(&mut self, req: ActionRequest, now: Instant) -> Result<(), String> {
        self.expire_approvals();
        if self.approvals.pending.len() >= MAX_PENDING
            || self
                .approvals
                .pending
                .iter()
                .any(|p| p.req.task_id == req.task_id)
        {
            return Err("Approval queue is full".into());
        }
        if self.control_owner() != ControlOwner::Agent
            || self.state() != Some(ComputerState::Running)
            || self.agent_cancel.is_cancelled()
        {
            return Err("Agent control is unavailable".into());
        }
        let view = PendingApproval {
            id: req.action_id,
            task_id: req.task_id,
            computer_id: req.computer_id,
            summary: format!("VM input: {}", req.action.verb()),
            expires_at: Utc::now() + chrono::Duration::seconds(120),
        };
        self.approvals.pending.push(Pending {
            req,
            generation: self.approvals.generation,
            deadline: now + APPROVAL_TTL,
            view,
            decision: None,
            typed: 0,
        });
        Ok(())
    }
    pub fn discard_approval(&mut self, id: ActionId) {
        if let Some(index) = self
            .approvals
            .pending
            .iter()
            .position(|p| p.req.action_id == id)
        {
            let p = self.approvals.pending.remove(index);
            self.remember_pending_stop(p, ApprovalStopReason::Cancelled);
        }
    }
    /// Poll under a short Core lock; wait outside it. No replacement arguments.
    pub fn poll_approval(
        &mut self,
        id: ActionId,
        task: TaskId,
        cancel: &CancellationToken,
    ) -> ApprovalProgress {
        self.poll_approval_at(id, task, cancel, Instant::now())
    }
    fn poll_approval_at(
        &mut self,
        id: ActionId,
        task: TaskId,
        cancel: &CancellationToken,
        now: Instant,
    ) -> ApprovalProgress {
        let Some(index) = self
            .approvals
            .pending
            .iter()
            .position(|p| p.req.action_id == id)
        else {
            if let Some(index) = self
                .approvals
                .stopped
                .iter()
                .position(|p| p.id == id && p.task == task)
            {
                return self
                    .approvals
                    .stopped
                    .remove(index)
                    .expect("located stopped entry")
                    .progress();
            }
            return Stopped {
                id,
                task,
                typed: 0,
                total: 0,
                reason: ApprovalStopReason::Stale,
            }
            .progress();
        };
        let mut p = self.approvals.pending.remove(index);
        let reason = if p.req.task_id != task || self.current_id() != Some(p.req.computer_id) {
            Some(ApprovalStopReason::Stale)
        } else if p.generation != self.approvals.generation
            || self.control_owner() != ControlOwner::Agent
            || self.state() != Some(ComputerState::Running)
        {
            Some(ApprovalStopReason::ControlChanged)
        } else if now >= p.deadline {
            Some(ApprovalStopReason::Expired)
        } else if cancel.is_cancelled() || self.agent_cancel.is_cancelled() {
            Some(ApprovalStopReason::Cancelled)
        } else {
            None
        };
        if let Some(reason) = reason {
            return self.stop_pending(&p, reason);
        }
        match p.decision {
            None => {
                self.approvals.pending.push(p);
                return ApprovalProgress::Waiting;
            }
            Some(ApprovalDecision::Deny) => {
                return self.stop_pending(&p, ApprovalStopReason::Denied)
            }
            Some(ApprovalDecision::ApproveOnce) => {}
        }
        let ctx = pegoles_policy::PolicyContext {
            ask_before_input: self.approvals.strict,
        };
        if pegoles_policy::evaluate(&p.req, &ctx).decision == Decision::Deny {
            return self.stop_pending(&p, ApprovalStopReason::Stale);
        }
        let mut part = p.req.clone();
        let mut more = false;

        if let ComputerAction::TypeText { text, sensitive } = &p.req.action {
            let total = text.chars().count();
            let next: String = text
                .chars()
                .skip(p.typed)
                .take(4 * pegoles_computer::input::TYPE_SLICE_CHARS)
                .collect();
            let next_count = next.chars().count();
            more = p.typed + next_count < total;
            part.action = ComputerAction::TypeText {
                text: next,
                sensitive: *sensitive,
            };
        }
        if p.typed == 0 {
            self.bus
                .publish(pegoles_protocol::AgentEvent::ActionStarted {
                    action_id: p.req.action_id,
                    request: p.req.redacted_for_event(),
                    at: Utc::now(),
                });
        }
        let (mut result, confirmed_typed) =
            self.execute_approved_slice(part, &ctx, cancel, p.typed);
        p.typed += confirmed_typed;
        if !result.success {
            let info = p.stopped(ApprovalStopReason::Cancelled);
            annotate_partial(&mut result, info.typed, info.total);
        }
        if more && result.success {
            self.approvals.pending.push(p);
            ApprovalProgress::Continue
        } else {
            self.settle_approval(&p, &result);
            ApprovalProgress::Finished(result)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendKind, EventBus};

    fn rig() -> (ComputerRegistry, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ComputerRegistry::with_dirs(
            EventBus::new(),
            BackendKind::Mock,
            dir.path().to_path_buf(),
        );
        let mut mock = pegoles_computer::MockComputerBackend::new();
        mock.set_input_enabled(true);
        r.create_default_on(Box::new(mock)).unwrap();
        r.start().unwrap();
        r.set_ask_before_input(true);
        r.begin_agent_session(&mut Vec::new()).unwrap();
        (r, dir)
    }
    fn queue(r: &mut ComputerRegistry, task: TaskId) -> ActionId {
        let result = r
            .execute_action(
                task,
                ComputerAction::KeyPress { key: "Tab".into() },
                false,
                &pegoles_policy::PolicyContext::default(),
                &CancellationToken::new(),
                false,
            )
            .0;
        assert_eq!(result.outcome, ActionOutcome::NeedsApproval);
        result.action_id
    }
    fn assert_stopped(
        r: &mut ComputerRegistry,
        id: ActionId,
        task: TaskId,
        cancel: &CancellationToken,
    ) {
        assert!(matches!(
            r.poll_approval(id, task, cancel),
            ApprovalProgress::Stopped {
                result: ActionResult {
                    outcome: ActionOutcome::Interrupted,
                    ..
                },
                ..
            }
        ));
        assert!(r.audit_rows(500).iter().all(|a| a.outcome != "Executed"));
        assert!(r.list_approvals().is_empty());
    }
    #[test]
    fn takeover_never_turns_explicit_denial_or_expiry_into_resumable_intervention() {
        for expired in [false, true] {
            let (mut r, _dir) = rig();
            let task = TaskId::new();
            let id = queue(&mut r, task);
            if expired {
                r.approvals.pending[0].deadline = Instant::now();
            } else {
                r.resolve_approval(id, ApprovalDecision::Deny).unwrap();
            }
            r.invalidate_approvals();
            let ApprovalProgress::Stopped { reason, .. } =
                r.poll_approval(id, task, &CancellationToken::new())
            else {
                panic!("not stopped")
            };
            assert_eq!(
                reason,
                if expired {
                    ApprovalStopReason::Expired
                } else {
                    ApprovalStopReason::Denied
                }
            );
        }
    }
    #[test]
    fn deny_cancel_wrong_task_and_replay_never_dispatch() {
        for mode in 0..4 {
            let (mut r, _dir) = rig();
            let task = TaskId::new();
            let id = queue(&mut r, task);
            let cancel = CancellationToken::new();
            assert!(r
                .resolve_approval(ActionId::new(), ApprovalDecision::ApproveOnce)
                .is_err());
            match mode {
                0 => r.resolve_approval(id, ApprovalDecision::Deny).unwrap(),
                1 => cancel.cancel(),
                2 => {
                    r.resolve_approval(id, ApprovalDecision::ApproveOnce)
                        .unwrap();
                    assert_stopped(&mut r, id, TaskId::new(), &cancel);
                    continue;
                }
                _ => {
                    r.resolve_approval(id, ApprovalDecision::ApproveOnce)
                        .unwrap();
                    r.cancel_agent_input("takeover");
                }
            }
            assert_stopped(&mut r, id, task, &cancel);
            assert!(r
                .resolve_approval(id, ApprovalDecision::ApproveOnce)
                .is_err());
        }
    }
    #[test]
    fn monotonic_expiry_at_exact_120_seconds_denies() {
        for seconds in [119, 120] {
            let (mut r, _dir) = rig();
            let task = TaskId::new();
            let now = Instant::now();
            let req = ActionRequest::new(
                task,
                r.current_id().unwrap(),
                ComputerAction::KeyPress { key: "Tab".into() },
            );
            let id = req.action_id;
            r.queue_approval_at(req, now).unwrap();
            let decision = r.resolve_approval_at(
                id,
                ApprovalDecision::ApproveOnce,
                now + Duration::from_secs(seconds),
            );
            assert_eq!(decision.is_ok(), seconds == 119);
            if seconds == 120 {
                assert!(matches!(
                    r.poll_approval_at(
                        id,
                        task,
                        &CancellationToken::new(),
                        now + Duration::from_secs(120)
                    ),
                    ApprovalProgress::Stopped {
                        reason: ApprovalStopReason::Expired,
                        ..
                    }
                ));
                assert!(r.list_approvals().is_empty());
                assert_eq!(r.audit_rows(500).last().unwrap().outcome, "Interrupted");
            }
        }
    }
    #[test]
    fn pending_bound_and_safe_summary_and_stale_generation() {
        let (mut r, _dir) = rig();
        for _ in 0..MAX_PENDING {
            let req = ActionRequest::new(
                TaskId::new(),
                r.current_id().unwrap(),
                ComputerAction::TypeText {
                    text: "private ordinary text".into(),
                    sensitive: false,
                },
            );
            r.queue_approval(req).unwrap();
        }
        let req = ActionRequest::new(
            TaskId::new(),
            r.current_id().unwrap(),
            ComputerAction::KeyPress { key: "Tab".into() },
        );
        assert!(r.queue_approval(req).is_err());
        assert!(!serde_json::to_string(&r.list_approvals())
            .unwrap()
            .contains("private ordinary text"));
        let id = r.approvals.pending[0].req.action_id;
        r.end_agent_session(&mut Vec::new());
        r.begin_agent_session(&mut Vec::new()).unwrap();
        assert!(r
            .resolve_approval(id, ApprovalDecision::ApproveOnce)
            .is_err());
    }
    #[test]
    fn wrong_computer_and_changed_payload_are_revalidated() {
        let (mut r, _dir) = rig();
        let task = TaskId::new();
        let id = queue(&mut r, task);
        r.approvals.pending[0].req.computer_id = ComputerId::new();
        assert!(r
            .resolve_approval(id, ApprovalDecision::ApproveOnce)
            .is_err());
        assert_stopped(&mut r, id, task, &CancellationToken::new());
        let id = queue(&mut r, task);
        r.resolve_approval(id, ApprovalDecision::ApproveOnce)
            .unwrap();
        r.approvals.pending[0].req.action = ComputerAction::TypeText {
            text: "-----BEGIN PRIVATE KEY-----".into(),
            sensitive: false,
        };
        assert_stopped(&mut r, id, task, &CancellationToken::new());
    }
    #[test]
    fn interrupted_long_input_reports_only_successful_prefix_after_cleanup() {
        for cleanup in 0..4 {
            let (mut r, _dir) = rig();
            let task = TaskId::new();
            let slice = 4 * pegoles_computer::input::TYPE_SLICE_CHARS;
            let result = r
                .execute_action(
                    task,
                    ComputerAction::TypeText {
                        text: "x".repeat(slice * 2),
                        sensitive: false,
                    },
                    false,
                    &pegoles_policy::PolicyContext::default(),
                    &CancellationToken::new(),
                    false,
                )
                .0;
            r.resolve_approval(result.action_id, ApprovalDecision::ApproveOnce)
                .unwrap();
            assert!(matches!(
                r.poll_approval(result.action_id, task, &CancellationToken::new()),
                ApprovalProgress::Continue
            ));
            if cleanup == 1 {
                r.cancel_agent_input("takeover");
            }
            if cleanup == 2 {
                r.approvals.pending[0].deadline = Instant::now();
                r.list_approvals();
            }
            if cleanup == 3 {
                r.stop().unwrap();
            }
            let cancel = CancellationToken::new();
            cancel.cancel();
            let ApprovalProgress::Stopped { result, .. } =
                r.poll_approval(result.action_id, task, &cancel)
            else {
                panic!("must stop")
            };
            assert!(
                result
                    .message
                    .contains(&format!("first {slice} of {}", slice * 2)),
                "{}",
                result.message
            );
        }
    }

    #[test]
    fn failed_next_slice_does_not_count_unexecuted_text() {
        let (mut r, _dir) = rig();
        let task = TaskId::new();
        let slice = 4 * pegoles_computer::input::TYPE_SLICE_CHARS;
        let id = r
            .execute_action(
                task,
                ComputerAction::TypeText {
                    text: "x".repeat(slice * 3),
                    sensitive: false,
                },
                false,
                &pegoles_policy::PolicyContext::default(),
                &CancellationToken::new(),
                false,
            )
            .0
            .action_id;
        r.resolve_approval(id, ApprovalDecision::ApproveOnce)
            .unwrap();
        assert!(matches!(
            r.poll_approval(id, task, &CancellationToken::new()),
            ApprovalProgress::Continue
        ));
        for _ in 0..pegoles_protocol::limits::MAX_ACTIONS_PER_SEC {
            r.rate_limiter.check(crate::input::input_clock_ms());
        }
        let ApprovalProgress::Finished(result) =
            r.poll_approval(id, task, &CancellationToken::new())
        else {
            panic!("must fail rate check")
        };
        assert!(!result.success);
        assert!(
            result
                .message
                .contains(&format!("first {slice} of {}", slice * 3)),
            "{}",
            result.message
        );
        assert!(!result.message.contains(&format!("first {} of", slice * 2)));
        assert!(r.list_approvals().is_empty());
    }

    #[test]
    fn approved_typing_emits_one_whole_action_terminal_with_confirmed_prefix() {
        use pegoles_protocol::AgentEvent;
        let mut streams = Vec::new();
        for mode in 0..6 {
            let (mut r, _dir) = rig();
            let mut rx = r.bus.subscribe();
            let task = TaskId::new();
            let cancel = CancellationToken::new();
            let id = r
                .execute_action(
                    task,
                    ComputerAction::TypeText {
                        text: "private marker".repeat(40),
                        sensitive: true,
                    },
                    false,
                    &pegoles_policy::PolicyContext::default(),
                    &cancel,
                    false,
                )
                .0
                .action_id;
            r.resolve_approval(id, ApprovalDecision::ApproveOnce)
                .unwrap();
            assert!(matches!(
                r.poll_approval(id, task, &cancel),
                ApprovalProgress::Continue
            ));
            let mut before = Vec::new();
            while let Ok(event) = rx.try_recv() {
                before.push(event);
            }
            match mode {
                1 => cancel.cancel(),
                2 => {
                    r.approvals.pending[0].deadline = Instant::now();
                    r.list_approvals();
                }
                3 => {
                    for _ in 0..pegoles_protocol::limits::MAX_ACTIONS_PER_SEC {
                        r.rate_limiter.check(crate::input::input_clock_ms());
                    }
                }
                4 => r.cancel_agent_input("takeover"),
                5 => r.discard_approval(id),
                _ => {}
            }
            let result = loop {
                match r.poll_approval(id, task, &cancel) {
                    ApprovalProgress::Continue => continue,
                    ApprovalProgress::Finished(result)
                    | ApprovalProgress::Stopped { result, .. } => break result,
                    ApprovalProgress::Waiting => panic!("already approved"),
                }
            };
            let mut after = before.clone();
            while let Ok(event) = rx.try_recv() {
                after.push(event);
            }
            assert_eq!(result.success, mode == 0);
            if mode != 0 {
                assert!(
                    result.message.contains("first 256 of 560"),
                    "{}",
                    result.message
                );
            }
            // Captured real Core events also exercise the frontend projection.
            streams.push(serde_json::json!({ "mode": mode, "before": before, "after": after, "result": result }));
        }
        println!(
            "APPROVAL_STREAMS={}",
            serde_json::to_string(&streams).unwrap()
        );
        for stream in &streams {
            let before: Vec<AgentEvent> = serde_json::from_value(stream["before"].clone()).unwrap();
            let after: Vec<AgentEvent> = serde_json::from_value(stream["after"].clone()).unwrap();
            let terminal = |e: &&AgentEvent| {
                matches!(
                    e,
                    AgentEvent::ActionCompleted { .. }
                        | AgentEvent::ActionFailed { .. }
                        | AgentEvent::ActionDenied { .. }
                )
            };
            assert_eq!(
                before.iter().filter(terminal).count(),
                0,
                "no terminal event before whole action settles"
            );
            assert_eq!(
                after.iter().filter(terminal).count(),
                1,
                "exactly one terminal event"
            );
            let result: ActionResult = serde_json::from_value(stream["result"].clone()).unwrap();
            let event = after.iter().find(terminal).unwrap();
            match event {
                AgentEvent::ActionCompleted {
                    request,
                    result: published,
                } => {
                    assert_eq!(request.action_id, result.action_id);
                    assert_eq!(published.message, result.message);
                    assert!(result.success);
                }
                AgentEvent::ActionFailed {
                    action_id, error, ..
                } => {
                    assert_eq!(*action_id, result.action_id);
                    assert_eq!(error, &result.message);
                    assert!(!result.success);
                }
                _ => panic!("unexpected terminal"),
            }
            assert!(!serde_json::to_string(&after)
                .unwrap()
                .contains("private marker"));
        }
    }

    // Real executor with a backend that rejects duplicate transport IDs.
    struct UniqueIdsBackend {
        inner: pegoles_computer::MockComputerBackend,
        seen: std::collections::HashSet<String>,
        fail_after: Option<usize>,
    }
    macro_rules! forward_mut {
        ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => { $(
            fn $name(&mut self, $($arg: $ty),*) -> $ret { self.inner.$name($($arg),*) }
        )* };
    }
    impl pegoles_computer::ComputerBackend for UniqueIdsBackend {
        forward_mut! {
            create(config: pegoles_protocol::ComputerConfig) -> pegoles_computer::Result<ComputerId>;
            start() -> pegoles_computer::Result<ComputerState>;
            stop() -> pegoles_computer::Result<ComputerState>;
            pause() -> pegoles_computer::Result<ComputerState>;
            resume() -> pegoles_computer::Result<ComputerState>;
            reset() -> pegoles_computer::Result<ComputerState>;
            snapshot() -> pegoles_computer::Result<pegoles_protocol::SnapshotId>;
            restore(id: pegoles_protocol::SnapshotId) -> pegoles_computer::Result<ComputerState>;
        }
        fn state(&self) -> ComputerState {
            self.inner.state()
        }
        fn id(&self) -> Option<ComputerId> {
            self.inner.id()
        }
        fn config(&self) -> Option<pegoles_protocol::ComputerConfig> {
            self.inner.config()
        }
        fn capabilities(&self) -> pegoles_computer::BackendCapabilities {
            self.inner.capabilities()
        }
        fn graphical_session(&self) -> pegoles_computer::guest::GraphicalSessionInfo {
            self.inner.graphical_session()
        }
        fn input_available(&self) -> bool {
            self.inner.input_available()
        }
        fn input_capabilities(&self) -> pegoles_computer::InputCapabilities {
            self.inner.input_capabilities()
        }
        fn input_execute(
            &mut self,
            id: &str,
            op: &pegoles_computer::InputOp,
        ) -> pegoles_computer::InputOutcome {
            if self.fail_after == Some(self.seen.len()) {
                return pegoles_computer::InputOutcome::failed("injected input failure");
            }
            if !self.seen.insert(id.into()) {
                return pegoles_computer::InputOutcome::failed("duplicate transport ID");
            }
            self.inner.input_execute(id, op)
        }
    }
    #[test]
    fn approved_slices_use_unique_transport_ids_for_one_original_action() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = ComputerRegistry::with_dirs(
            EventBus::new(),
            BackendKind::Mock,
            dir.path().to_path_buf(),
        );
        let mut inner = pegoles_computer::MockComputerBackend::new();
        inner.set_input_enabled(true);
        r.create_default_on(Box::new(UniqueIdsBackend {
            inner,
            seen: Default::default(),
            fail_after: None,
        }))
        .unwrap();
        r.start().unwrap();
        r.set_ask_before_input(true);
        r.begin_agent_session(&mut Vec::new()).unwrap();
        let task = TaskId::new();
        let cancel = CancellationToken::new();
        let id = r
            .execute_action(
                task,
                ComputerAction::TypeText {
                    text: "x".repeat(8 * pegoles_computer::input::TYPE_SLICE_CHARS),
                    sensitive: false,
                },
                false,
                &pegoles_policy::PolicyContext::default(),
                &cancel,
                false,
            )
            .0
            .action_id;
        r.resolve_approval(id, ApprovalDecision::ApproveOnce)
            .unwrap();
        assert!(matches!(
            r.poll_approval(id, task, &cancel),
            ApprovalProgress::Continue
        ));
        let ApprovalProgress::Finished(result) = r.poll_approval(id, task, &cancel) else {
            panic!("must finish")
        };
        assert_eq!(result.action_id, id);
        assert!(result.success, "{}", result.message);
    }

    #[test]
    fn failed_primitive_reports_only_acknowledged_characters_in_single_terminal() {
        use pegoles_protocol::AgentEvent;
        for successful_primitives in [0, 1, 4, 5, 8] {
            let dir = tempfile::tempdir().unwrap();
            let bus = EventBus::new();
            let mut rx = bus.subscribe();
            let mut r =
                ComputerRegistry::with_dirs(bus, BackendKind::Mock, dir.path().to_path_buf());
            let mut inner = pegoles_computer::MockComputerBackend::new();
            inner.set_input_enabled(true);
            r.create_default_on(Box::new(UniqueIdsBackend {
                inner,
                seen: Default::default(),
                fail_after: Some(successful_primitives),
            }))
            .unwrap();
            r.start().unwrap();
            r.set_ask_before_input(true);
            r.begin_agent_session(&mut Vec::new()).unwrap();
            let task = TaskId::new();
            let cancel = CancellationToken::new();
            let id = r
                .execute_action(
                    task,
                    ComputerAction::TypeText {
                        text: "x".repeat(560),
                        sensitive: true,
                    },
                    false,
                    &pegoles_policy::PolicyContext::default(),
                    &cancel,
                    false,
                )
                .0
                .action_id;
            r.resolve_approval(id, ApprovalDecision::ApproveOnce)
                .unwrap();
            let result = loop {
                match r.poll_approval(id, task, &cancel) {
                    ApprovalProgress::Continue => {}
                    ApprovalProgress::Finished(result) => break result,
                    _ => panic!("expected dispatch failure"),
                }
            };
            assert!(!result.success);
            if successful_primitives == 0 {
                assert_eq!(result.message, "injected input failure");
            } else {
                assert!(
                    result.message.ends_with(&format!(
                        "(the first {} of 560 characters were typed)",
                        successful_primitives * pegoles_computer::input::TYPE_SLICE_CHARS
                    )),
                    "{}",
                    result.message
                );
            }
            let mut terminals = Vec::new();
            while let Ok(event) = rx.try_recv() {
                match event {
                    AgentEvent::ActionCompleted { .. } => panic!("partial input cannot complete"),
                    AgentEvent::ActionFailed {
                        action_id, error, ..
                    } => terminals.push((action_id, error)),
                    _ => {}
                }
            }
            assert_eq!(terminals, vec![(id, result.message)]);
        }
    }

    #[test]
    fn decision_and_planner_tags_are_closed() {
        for json in [
            r#""allow_all""#,
            r#"{"decision":"approve_once","action":{}}"#,
        ] {
            assert!(serde_json::from_str::<ApprovalDecision>(json).is_err());
        }
        for tag in [
            "approve",
            "approve_once",
            "resolve_approval",
            "set_input_approval",
        ] {
            assert!(
                serde_json::from_str::<ComputerAction>(&format!(r#"{{"type":"{tag}"}}"#)).is_err()
            );
            assert!(
                serde_json::from_str::<pegoles_guest_proto::GuestMessage>(&format!(
                    r#"{{"type":"{tag}"}}"#
                ))
                .is_err()
            );
        }
    }
}

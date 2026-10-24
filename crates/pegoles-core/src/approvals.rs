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
    fn progress(self) -> ApprovalProgress {
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
            self.remember_stop(p.stopped(ApprovalStopReason::ControlChanged));
        }
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
                self.remember_stop(p.stopped(ApprovalStopReason::Expired));
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
            self.remember_stop(p.stopped(ApprovalStopReason::Cancelled));
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
            return p.stopped(reason).progress();
        }
        match p.decision {
            None => {
                self.approvals.pending.push(p);
                return ApprovalProgress::Waiting;
            }
            Some(ApprovalDecision::Deny) => {
                return p.stopped(ApprovalStopReason::Denied).progress()
            }
            Some(ApprovalDecision::ApproveOnce) => {}
        }
        let ctx = pegoles_policy::PolicyContext {
            ask_before_input: self.approvals.strict,
        };
        if pegoles_policy::evaluate(&p.req, &ctx).decision == Decision::Deny {
            return p.stopped(ApprovalStopReason::Stale).progress();
        }
        let mut part = p.req.clone();
        let mut more = false;
        let mut next_count = 0;
        if let ComputerAction::TypeText { text, sensitive } = &p.req.action {
            let total = text.chars().count();
            let next: String = text
                .chars()
                .skip(p.typed)
                .take(4 * pegoles_computer::input::TYPE_SLICE_CHARS)
                .collect();
            next_count = next.chars().count();
            more = p.typed + next_count < total;
            part.action = ComputerAction::TypeText {
                text: next,
                sensitive: *sensitive,
            };
        }
        let mut result = self
            .execute_request(part, &ctx, cancel, false, Some(p.typed))
            .0;
        if result.success {
            p.typed += next_count;
        } else {
            let info = p.stopped(ApprovalStopReason::Cancelled);
            annotate_partial(&mut result, info.typed, info.total);
        }
        if more && result.success {
            self.approvals.pending.push(p);
            ApprovalProgress::Continue
        } else {
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
                assert!(r.audit_rows(500).is_empty());
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

    // Real executor with a backend that rejects duplicate transport IDs.
    struct UniqueIdsBackend {
        inner: pegoles_computer::MockComputerBackend,
        seen: std::collections::HashSet<String>,
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

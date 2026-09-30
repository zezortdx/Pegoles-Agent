//! [`AgentComputer`] over Pegoles Core, behind whatever lock the host
//! uses. The desktop app and the hardware E2E harness share this exact
//! code, so the E2E proves the product path.
//!
//! Lock discipline: Core is only held for one action, one observation,
//! or one readiness probe at a time. Waits sleep with the lock released,
//! and long typed text is dispatched as several short actions with the
//! lock released between them, so status polling, guest heartbeats and
//! cancellation never stall behind an agent run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pegoles_core::{
    validate_internet, CancellationToken, ComputerRegistry, EventBus, TaskManager,
    HANDSHAKE_TIMEOUT,
};
use pegoles_policy::PolicyContext;
use pegoles_protocol::{
    ActionId, ActionOutcome, ActionRequest, ActionResult, AgentEvent, ComputerAction, ComputerId,
    ComputerState, ControlOwner, Decision, GuestRuntimeState, InternetAccess, TaskId, TaskStatus,
};

use crate::planner::Screenshot;
use crate::runner::AgentComputer;

/// Characters of typed text dispatched per Core lock: a few guest
/// primitives (about 2 s of paced keystrokes), not the 30-60 s a full
/// `MAX_TYPE_CHARS` action takes.
pub const TYPE_CHARS_PER_LOCK: usize = 4 * pegoles_computer::input::TYPE_SLICE_CHARS;

/// The person's native confirmation for a task's internet access. Runs on
/// the run's thread with Core unlocked, may block on a dialog. `Ok(false)`
/// (declined) and `Err` (could not ask) both leave the task offline.
pub type InternetConfirm = Arc<dyn Fn(&InternetAccess) -> Result<bool, String> + Send + Sync>;

/// What a task asked for and how it gets confirmed (docs/EGRESS.md).
#[derive(Clone)]
pub struct InternetPlan {
    pub task: TaskId,
    pub access: InternetAccess,
    pub confirm: InternetConfirm,
}

/// Exclusive access to Core. Implementations recover a poisoned lock and
/// keep their own bookkeeping (e.g. event history) in sync afterwards.
pub trait CoreAccess: Send + Sync {
    fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R;
}

pub struct CoreComputer<A: CoreAccess> {
    access: A,
    bus: EventBus,
    ctx: PolicyContext,
    /// When the executor was last entered: actions, waits and
    /// observations all count against its rate brake, so all of them are
    /// paced to it instead of being rejected by it.
    last_act: std::sync::Mutex<Option<Instant>>,
    /// Budget for create/start + guest runtime readiness.
    pub prepare_timeout: Duration,
    /// Budget for the guest to answer once the internet channel is open.
    pub internet_timeout: Duration,
    internet: Option<InternetPlan>,
}

impl<A: CoreAccess> CoreComputer<A> {
    pub fn new(access: A, bus: EventBus) -> Self {
        Self {
            access,
            bus,
            ctx: PolicyContext::default(),
            last_act: std::sync::Mutex::new(None),
            prepare_timeout: Duration::from_secs(180),
            internet_timeout: HANDSHAKE_TIMEOUT,
            internet: None,
        }
    }

    /// This run's task asked for internet: confirm it natively, then open
    /// it once the agent holds the computer. Without a plan (or with mode
    /// off) nothing is ever asked or opened.
    pub fn with_internet(mut self, plan: InternetPlan) -> Self {
        self.internet = Some(plan);
        self
    }

    /// Ask the person (before the computer boots, Core unlocked). `None`
    /// keeps the task offline, after telling the person why.
    fn confirm_internet(&self) -> Option<InternetAccess> {
        let plan = self.internet.as_ref().filter(|p| !p.access.is_off())?;
        let access = match validate_internet(&plan.access) {
            Ok((access, _)) => access,
            Err(why) => return self.offline(plan.task, &why),
        };
        match (plan.confirm)(&access) {
            Ok(true) => Some(access),
            Ok(false) => self.offline(plan.task, "it was not confirmed in the Pegoles window"),
            Err(why) => self.offline(plan.task, &why),
        }
    }

    /// Open the confirmed session and wait (Core unlocked) for the guest.
    /// Any failure: closed again, the task runs offline and says so.
    fn open_internet(&self, task: TaskId, access: &InternetAccess, cancel: &CancellationToken) {
        // Quick steps under Core's lock, every wait outside it (the helper
        // and the guest can take seconds; Core must stay usable).
        let opened = self
            .access
            .with_core(|r, _| r.egress_prepare(task, access))
            .and_then(|mut opening| {
                let endpoint = opening.wait();
                self.access
                    .with_core(|r, _| r.egress_install(opening, endpoint))
            });
        let result = opened.and_then(|handshake| handshake.wait(cancel, self.internet_timeout));
        if let Err(why) = result {
            self.access
                .with_core(|r, _| r.egress_close("the internet channel did not come up"));
            if !cancel.is_cancelled() {
                self.offline(task, &why);
            }
        }
    }

    /// The task runs offline although internet was asked for.
    fn offline(&self, task: TaskId, why: &str) -> Option<InternetAccess> {
        let why: String = why
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(300)
            .collect();
        let at = chrono::Utc::now();
        self.bus.publish(AgentEvent::InternetUnavailable {
            task_id: task,
            reason: why.clone(),
            at,
        });
        self.bus.publish(AgentEvent::AgentMessage {
            task_id: task,
            kind: pegoles_protocol::AgentMessageKind::Progress,
            text: format!("Internet access is off for this task: {why}. It continues offline."),
            at,
        });
        None
    }

    /// Keep input at or below the executor's rate brake (sleeping with
    /// Core unlocked), so a fast batch is slowed down, not failed.
    fn pace(&self) {
        let gap =
            Duration::from_millis(1_000 / pegoles_protocol::limits::MAX_ACTIONS_PER_SEC as u64 + 1);
        let mut last = self.last_act.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = *last {
            let since = prev.elapsed();
            if since < gap {
                std::thread::sleep(gap - since);
            }
        }
        *last = Some(Instant::now());
    }

    fn interrupted(reason: &str) -> ActionResult {
        ActionResult::failed(
            ActionId::new(),
            chrono::Utc::now(),
            ActionOutcome::Interrupted,
            reason,
        )
    }

    /// One action through the executor under one short lock.
    fn execute(
        &self,
        task: TaskId,
        action: ComputerAction,
        cancel: &CancellationToken,
    ) -> ActionResult {
        self.pace();
        self.access.with_core(|r, _| {
            r.execute_action(task, action, false, &self.ctx, cancel, false)
                .0
        })
    }

    /// Long text as consecutive executor actions, one lock each: every
    /// part is rate-limited, policy-checked, ownership-checked and
    /// audited on its own, and Stop or a takeover lands between parts.
    /// Policy judges the WHOLE text first (the key-material tripwire
    /// must see strings a part boundary would cut); text it would refuse
    /// goes through the executor unsplit, so the denial is recorded
    /// exactly as before.
    fn type_in_parts(
        &self,
        task: TaskId,
        text: String,
        sensitive: bool,
        cancel: &CancellationToken,
    ) -> ActionResult {
        // Policy judges an action's shape only; the ids are placeholders.
        let whole = ActionRequest::new(
            task,
            ComputerId::new(),
            ComputerAction::TypeText {
                text: text.clone(),
                sensitive,
            },
        );
        if pegoles_policy::evaluate(&whole, &self.ctx).decision != Decision::Allow {
            return self.execute(task, whole.action, cancel);
        }
        let chars: Vec<char> = text.chars().collect();
        let total = chars.len();
        let mut typed = 0;
        let mut last = None;
        for part in chars.chunks(TYPE_CHARS_PER_LOCK) {
            if typed > 0 && cancel.is_cancelled() {
                return partial(Self::interrupted("cancelled mid-action"), typed, total);
            }
            let text = part.iter().collect();
            let result = self.execute(task, ComputerAction::TypeText { text, sensitive }, cancel);
            if result.outcome != ActionOutcome::Executed {
                return partial(result, typed, total);
            }
            typed += part.len();
            last = Some(result);
        }
        // Only called for text longer than one part: some part ran.
        last.unwrap_or_else(|| Self::interrupted("nothing was typed"))
    }
}

/// A failure after some text was already typed says how much, so the
/// planner does not type it all again.
fn partial(mut result: ActionResult, typed: usize, total: usize) -> ActionResult {
    if typed > 0 {
        result.message = format!(
            "{} (the first {typed} of {total} characters were typed)",
            result.message
        );
    }
    result
}

enum Readiness {
    Ready,
    Waiting,
    Broken(String),
}

impl<A: CoreAccess> AgentComputer for CoreComputer<A> {
    fn prepare(&self, cancel: &CancellationToken) -> Result<(), String> {
        let internet = self.confirm_internet();
        // Create / boot / resume as needed (one lifecycle call per lock).
        self.access.with_core(|r, _| -> Result<(), String> {
            if !r.is_created() {
                r.create_default().map_err(|e| e.to_string())?;
            }
            match r.state() {
                Some(ComputerState::Stopped) => {
                    r.start().map_err(|e| e.to_string())?;
                }
                Some(ComputerState::Paused) => {
                    r.resume().map_err(|e| e.to_string())?;
                }
                Some(ComputerState::Error) => {
                    return Err("the computer is in an error state; restart it".to_string())
                }
                _ => {}
            }
            Ok(())
        })?;
        let deadline = Instant::now() + self.prepare_timeout;
        loop {
            if cancel.is_cancelled() {
                return Err("cancelled".to_string());
            }
            let readiness = self.access.with_core(|r, _| {
                r.pump();
                if r.state() == Some(ComputerState::Error) {
                    return Readiness::Broken("the computer failed".to_string());
                }
                match r.guest_state() {
                    GuestRuntimeState::Incompatible => {
                        return Readiness::Broken("incompatible guest runtime".to_string())
                    }
                    GuestRuntimeState::Error => {
                        return Readiness::Broken("the guest runtime reported an error".to_string())
                    }
                    _ => {}
                }
                if r.input_available() && r.input_status().frame_available {
                    Readiness::Ready
                } else {
                    Readiness::Waiting
                }
            });
            match readiness {
                Readiness::Ready => break,
                Readiness::Broken(reason) => return Err(reason),
                Readiness::Waiting if Instant::now() >= deadline => {
                    return Err(format!(
                        "the guest was not ready within {} s",
                        self.prepare_timeout.as_secs()
                    ))
                }
                Readiness::Waiting => std::thread::sleep(Duration::from_millis(250)),
            }
        }
        // The run owns agent control until it ends (a human takeover
        // revokes it and stops the run; see `act`).
        self.access.with_core(|r, _| {
            let mut out = Vec::new();
            r.begin_agent_session(&mut out)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })?;
        if let (Some(access), Some(plan)) = (internet, self.internet.as_ref()) {
            self.open_internet(plan.task, &access, cancel);
        }
        Ok(())
    }

    fn observe(&self, task: TaskId, cancel: &CancellationToken) -> Result<Screenshot, String> {
        self.pace();
        let (meta, rgba) = self.access.with_core(|r, _| {
            let (result, _) = r.execute_action(
                task,
                ComputerAction::ObserveScreen,
                false,
                &self.ctx,
                cancel,
                false,
            );
            if !result.success {
                return Err(result.message);
            }
            r.last_frame_bytes()
                .ok_or_else(|| "no frame captured".to_string())
        })?;
        // Encode with the lock released.
        let png = pegoles_computer::encode_png_rgb_fast(meta.width_px, meta.height_px, &rgba)
            .map_err(|e| e.to_string())?;
        Ok(Screenshot {
            png,
            width: meta.width_px,
            height: meta.height_px,
        })
    }

    fn act(
        &self,
        task: TaskId,
        action: ComputerAction,
        cancel: &CancellationToken,
    ) -> ActionResult {
        let owns_control = self
            .access
            .with_core(|r, _| r.control_owner() == ControlOwner::Agent);
        if !owns_control {
            // The human took the computer (or it paused/stopped): the run
            // ends here. The executor re-checks ownership under the same
            // lock as the dispatch, so this early exit is only a fast path.
            cancel.cancel();
            return Self::interrupted("the agent no longer controls the computer");
        }
        if let ComputerAction::Wait { duration_ms } = action {
            self.pace();
            let ticket = match self
                .access
                .with_core(|r, _| r.begin_wait(task, duration_ms, &self.ctx))
            {
                Ok(t) => t,
                Err(result) => return result,
            };
            let deadline = Instant::now() + Duration::from_millis(ticket.duration_ms() as u64);
            let mut interrupted = false;
            while Instant::now() < deadline {
                if cancel.is_cancelled() {
                    interrupted = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50).min(deadline - Instant::now()));
            }
            return self
                .access
                .with_core(|r, _| r.end_wait(ticket, interrupted));
        }
        let result = match action {
            ComputerAction::TypeText { text, sensitive }
                if text.chars().count() > TYPE_CHARS_PER_LOCK =>
            {
                self.type_in_parts(task, text, sensitive, cancel)
            }
            action => self.execute(task, action, cancel),
        };
        if result.outcome == ActionOutcome::Interrupted {
            cancel.cancel();
        }
        result
    }

    fn set_status(&self, task: TaskId, status: TaskStatus) -> Result<(), String> {
        self.access.with_core(|r, tasks| {
            let result = match status {
                TaskStatus::Running => tasks.start_task(&task),
                TaskStatus::Completed => tasks.finish_task(&task),
                TaskStatus::Failed => tasks.fail_task(&task),
                TaskStatus::Cancelled => tasks.cancel_task(&task),
                TaskStatus::WaitingForApproval => tasks.wait_for_approval(&task),
                TaskStatus::Pending => return Err("a task cannot return to pending".to_string()),
            };
            if matches!(
                status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                let mut out = Vec::new();
                r.end_agent_session(&mut out);
            }
            result.map(|_| ()).map_err(|e| e.to_string())
        })
    }

    fn publish(&self, event: AgentEvent) {
        self.bus.publish(event);
    }

    fn internet(&self) -> Option<InternetAccess> {
        let status = self.access.with_core(|r, _| r.egress_status());
        status.active.then_some(InternetAccess {
            mode: status.mode,
            domains: status.domains,
        })
    }
}

#[cfg(test)]
mod internet_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_task, PlannedCall, RunEnd, RunLimits, ScriptedPlanner, Step};
    use pegoles_core::BackendKind;
    use pegoles_protocol::PointerButton;
    use std::sync::{Arc, Mutex};

    struct Shared(Arc<Mutex<(ComputerRegistry, TaskManager)>>);

    impl CoreAccess for Shared {
        fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R {
            let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let (r, t) = &mut *guard;
            f(r, t)
        }
    }

    /// Registry with a created (stopped) Mock computer whose input and
    /// capture planes are enabled. `prepare` must still start it.
    fn rig() -> (
        Arc<Mutex<(ComputerRegistry, TaskManager)>>,
        EventBus,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let bus = EventBus::new();
        let mut registry =
            ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.path().to_path_buf());
        let mut mock = pegoles_computer::MockComputerBackend::new();
        mock.set_input_enabled(true);
        registry.create_default_on(Box::new(mock)).unwrap();
        let tasks = TaskManager::new(bus.clone());
        (Arc::new(Mutex::new((registry, tasks))), bus, dir)
    }

    fn computer(
        core: &Arc<Mutex<(ComputerRegistry, TaskManager)>>,
        bus: EventBus,
    ) -> CoreComputer<Shared> {
        let mut c = CoreComputer::new(Shared(core.clone()), bus);
        c.prepare_timeout = Duration::from_secs(3);
        c
    }

    #[test]
    fn scripted_run_goes_through_the_real_executor_and_policy() {
        let (core, bus, _dir) = rig();
        let mut events = bus.subscribe();
        let task = core
            .lock()
            .unwrap()
            .1
            .submit_task("click the middle")
            .unwrap()
            .id;
        let computer = computer(&core, bus.clone());
        let calls = vec![
            PlannedCall {
                call_id: "1".into(),
                label: "left_click".into(),
                steps: Ok(vec![Step::Act(ComputerAction::Click {
                    x: 0.5,
                    y: 0.5,
                    button: PointerButton::Primary,
                })]),
            },
            PlannedCall {
                call_id: "2".into(),
                label: "wait".into(),
                steps: Ok(vec![Step::Act(ComputerAction::Wait { duration_ms: 20 })]),
            },
            PlannedCall {
                call_id: "3".into(),
                label: "evil".into(),
                // Out of the unit square: policy must deny it.
                steps: Ok(vec![Step::Act(ComputerAction::MovePointer {
                    x: 9.0,
                    y: 0.0,
                })]),
            },
        ];
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let mut planner = ScriptedPlanner::new(vec![calls], |shot| {
            assert!(shot.width > 0 && !shot.png.is_empty());
            Ok("verified".to_string())
        })
        .observing(move |o| seen2.lock().unwrap().extend(o.to_vec()));
        let report = run_task(
            task,
            "click the middle",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Completed, "{}", report.summary);
        let outcomes = seen.lock().unwrap().clone();
        assert!(outcomes[0].result.is_ok(), "{:?}", outcomes[0]);
        assert!(outcomes[1].result.is_ok(), "{:?}", outcomes[1]);
        assert!(
            matches!(&outcomes[2].result, Err(e) if e.contains("Blocked by Pegoles policy")),
            "{:?}",
            outcomes[2]
        );
        let mut denied = 0;
        let mut completed = 0;
        loop {
            use tokio::sync::broadcast::error::TryRecvError;
            match events.try_recv() {
                Ok(AgentEvent::ActionDenied { .. }) => denied += 1,
                Ok(AgentEvent::ActionCompleted { .. }) => completed += 1,
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        assert_eq!(denied, 1, "policy denial must be visible in the stream");
        assert!(completed >= 3, "observe + click + wait complete");
        let guard = core.lock().unwrap();
        assert_eq!(guard.1.get(&task).unwrap().status, TaskStatus::Completed);
        assert_eq!(
            guard.0.control_owner(),
            ControlOwner::None,
            "control released"
        );
    }

    #[test]
    fn human_takeover_ends_the_run() {
        let (core, bus, _dir) = rig();
        let task = core.lock().unwrap().1.submit_task("x").unwrap().id;
        let computer = computer(&core, bus);
        let cancel = CancellationToken::new();
        computer.prepare(&cancel).unwrap();
        // Simulate the human grabbing control mid-run.
        core.lock().unwrap().0.cancel_agent_input("takeover");
        {
            let mut g = core.lock().unwrap();
            let mut out = Vec::new();
            g.0.end_agent_session(&mut out);
        }
        let result = computer.act(task, ComputerAction::KeyPress { key: "a".into() }, &cancel);
        assert_eq!(result.outcome, ActionOutcome::Interrupted);
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn waits_do_not_hold_core() {
        let (core, bus, _dir) = rig();
        let task = core.lock().unwrap().1.submit_task("x").unwrap().id;
        let computer = Arc::new(computer(&core, bus));
        let cancel = CancellationToken::new();
        computer.prepare(&cancel).unwrap();
        let c2 = computer.clone();
        let cancel2 = cancel.clone();
        let waiter = std::thread::spawn(move || {
            c2.act(task, ComputerAction::Wait { duration_ms: 1_500 }, &cancel2)
        });
        std::thread::sleep(Duration::from_millis(100));
        let t0 = Instant::now();
        let _ = core.lock().unwrap().0.state();
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "Core stayed locked during a wait"
        );
        cancel.cancel();
        let result = waiter.join().unwrap();
        assert_eq!(result.outcome, ActionOutcome::Interrupted);
    }

    #[test]
    fn observations_and_waits_are_paced_not_rate_limited() {
        // Found by the security matrix: after a sustained burst of paced
        // input, an unpaced screenshot hit the executor's rate brake and
        // the run failed on its final evidence frame.
        let (core, bus, _dir) = rig();
        let task = core.lock().unwrap().1.submit_task("x").unwrap().id;
        let computer = computer(&core, bus);
        let cancel = CancellationToken::new();
        computer.prepare(&cancel).unwrap();
        for _ in 0..10 {
            let r = computer.act(task, ComputerAction::KeyPress { key: "a".into() }, &cancel);
            assert_eq!(r.outcome, ActionOutcome::Executed, "{}", r.message);
            let w = computer.act(task, ComputerAction::Wait { duration_ms: 1 }, &cancel);
            assert_eq!(w.outcome, ActionOutcome::Executed, "{}", w.message);
            if let Err(e) = computer.observe(task, &cancel) {
                panic!("observation rejected: {e}");
            }
        }
    }

    /// Core behind its mutex, watched from outside: records the text each
    /// lock hold dispatched to the guest, and plays a human takeover once
    /// `take_over_after` parts were typed.
    struct Probe {
        core: Arc<Mutex<(ComputerRegistry, TaskManager)>>,
        events: Mutex<tokio::sync::broadcast::Receiver<AgentEvent>>,
        typed_per_hold: Mutex<Vec<Vec<String>>>,
        take_over_after: Option<usize>,
    }

    impl CoreAccess for Probe {
        fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R {
            let result = {
                let mut guard = self.core.lock().unwrap_or_else(|e| e.into_inner());
                let parts: usize = self
                    .typed_per_hold
                    .lock()
                    .unwrap()
                    .iter()
                    .map(Vec::len)
                    .sum();
                if self.take_over_after.is_some_and(|n| parts >= n) {
                    guard.0.cancel_agent_input("takeover");
                    guard.0.end_agent_session(&mut Vec::new());
                }
                let (r, t) = &mut *guard;
                f(r, t)
            };
            let mut typed = Vec::new();
            let mut rx = self.events.lock().unwrap();
            while let Ok(event) = rx.try_recv() {
                if let AgentEvent::ActionStarted { request, .. } = event {
                    if let ComputerAction::TypeText { text, .. } = request.action {
                        typed.push(text);
                    }
                }
            }
            if !typed.is_empty() {
                self.typed_per_hold.lock().unwrap().push(typed);
            }
            result
        }
    }

    fn probed(take_over_after: Option<usize>) -> (CoreComputer<Probe>, TaskId, tempfile::TempDir) {
        let (core, bus, dir) = rig();
        let task = core.lock().unwrap().1.submit_task("type").unwrap().id;
        let probe = Probe {
            core,
            events: Mutex::new(bus.subscribe()),
            typed_per_hold: Mutex::new(Vec::new()),
            take_over_after,
        };
        let mut computer = CoreComputer::new(probe, bus);
        computer.prepare_timeout = Duration::from_secs(3);
        computer.prepare(&CancellationToken::new()).unwrap();
        (computer, task, dir)
    }

    fn text_of(n: usize) -> String {
        (0..n).map(|i| (b'a' + (i % 26) as u8) as char).collect()
    }

    #[test]
    fn long_text_is_typed_in_parts_with_core_released_between_them() {
        let (computer, task, _dir) = probed(None);
        let text = text_of(1000);
        let result = computer.act(
            task,
            ComputerAction::TypeText {
                text: text.clone(),
                sensitive: false,
            },
            &CancellationToken::new(),
        );
        assert_eq!(
            result.outcome,
            ActionOutcome::Executed,
            "{}",
            result.message
        );
        let holds = computer.access.typed_per_hold.lock().unwrap().clone();
        // One part per lock hold, each at most TYPE_CHARS_PER_LOCK, in
        // order, nothing lost.
        assert_eq!(holds.len(), text.len().div_ceil(TYPE_CHARS_PER_LOCK));
        assert!(holds.iter().all(|h| h.len() == 1), "{holds:?}");
        assert!(holds
            .iter()
            .all(|h| h[0].chars().count() <= TYPE_CHARS_PER_LOCK));
        assert_eq!(holds.concat().concat(), text);

        // Short text stays one action.
        let (computer, task, _dir) = probed(None);
        let short = text_of(TYPE_CHARS_PER_LOCK);
        computer.act(
            task,
            ComputerAction::TypeText {
                text: short.clone(),
                sensitive: false,
            },
            &CancellationToken::new(),
        );
        assert_eq!(
            *computer.access.typed_per_hold.lock().unwrap(),
            vec![vec![short]]
        );
    }

    #[test]
    fn key_material_cut_by_a_part_boundary_is_still_refused() {
        let (computer, task, _dir) = probed(None);
        // "PRIVATE " ends the first part and "KEY-----" starts the second:
        // neither part alone trips the policy's tripwire.
        let marker = "-----BEGIN RSA PRIVATE KEY-----";
        let prefix = "a".repeat(TYPE_CHARS_PER_LOCK - 8 - marker.find("PRIVATE").unwrap());
        let text = format!("{prefix}{marker}\nMIIEow{}", "b".repeat(300));
        let first: String = text.chars().take(TYPE_CHARS_PER_LOCK).collect();
        assert!(first.ends_with("PRIVATE "), "{first}");
        let result = computer.act(
            task,
            ComputerAction::TypeText {
                text,
                sensitive: false,
            },
            &CancellationToken::new(),
        );
        assert_eq!(result.outcome, ActionOutcome::Blocked, "{}", result.message);
        assert!(
            computer.access.typed_per_hold.lock().unwrap().is_empty(),
            "nothing reached the guest"
        );
    }

    #[test]
    fn a_takeover_between_parts_stops_the_rest() {
        let (computer, task, _dir) = probed(Some(1));
        let cancel = CancellationToken::new();
        let result = computer.act(
            task,
            ComputerAction::TypeText {
                text: text_of(1000),
                sensitive: false,
            },
            &cancel,
        );
        assert_eq!(
            result.outcome,
            ActionOutcome::Interrupted,
            "{}",
            result.message
        );
        assert!(
            result
                .message
                .contains(&format!("first {TYPE_CHARS_PER_LOCK} of 1000")),
            "{}",
            result.message
        );
        let parts: usize = computer
            .access
            .typed_per_hold
            .lock()
            .unwrap()
            .iter()
            .map(Vec::len)
            .sum();
        assert_eq!(parts, 1);
        assert!(cancel.is_cancelled(), "the run ends");
    }
}

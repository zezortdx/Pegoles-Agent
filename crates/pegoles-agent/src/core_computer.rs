//! [`AgentComputer`] over Pegoles Core, behind whatever lock the host
//! uses. The desktop app and the hardware E2E harness share this exact
//! code, so the E2E proves the product path.
//!
//! Lock discipline: Core is only held for one action, one observation,
//! or one readiness probe at a time. Waits sleep with the lock released,
//! so status polling, guest heartbeats and cancellation never stall
//! behind an agent run.

use std::time::{Duration, Instant};

use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, TaskManager};
use pegoles_policy::PolicyContext;
use pegoles_protocol::{
    ActionId, ActionOutcome, ActionResult, AgentEvent, ComputerAction, ComputerState, ControlOwner,
    GuestRuntimeState, TaskId, TaskStatus,
};

use crate::planner::Screenshot;
use crate::runner::AgentComputer;

/// Exclusive access to Core. Implementations recover a poisoned lock and
/// keep their own bookkeeping (e.g. event history) in sync afterwards.
pub trait CoreAccess: Send + Sync {
    fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R;
}

pub struct CoreComputer<A: CoreAccess> {
    access: A,
    bus: EventBus,
    ctx: PolicyContext,
    /// When the last input action was dispatched: actions are paced to
    /// the executor's rate brake instead of being rejected by it.
    last_act: std::sync::Mutex<Option<Instant>>,
    /// Budget for create/start + guest runtime readiness.
    pub prepare_timeout: Duration,
}

impl<A: CoreAccess> CoreComputer<A> {
    pub fn new(access: A, bus: EventBus) -> Self {
        Self {
            access,
            bus,
            ctx: PolicyContext::default(),
            last_act: std::sync::Mutex::new(None),
            prepare_timeout: Duration::from_secs(180),
        }
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
}

enum Readiness {
    Ready,
    Waiting,
    Broken(String),
}

impl<A: CoreAccess> AgentComputer for CoreComputer<A> {
    fn prepare(&self, cancel: &CancellationToken) -> Result<(), String> {
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
        })
    }

    fn observe(&self, task: TaskId, cancel: &CancellationToken) -> Result<Screenshot, String> {
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
        self.pace();
        let result = self.access.with_core(|r, _| {
            r.execute_action(task, action, false, &self.ctx, cancel, false)
                .0
        });
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
}

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
}

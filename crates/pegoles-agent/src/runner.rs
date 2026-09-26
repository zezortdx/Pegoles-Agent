//! The task loop: observe → plan → act (through policy) → observe …
//!
//! Deterministic guard rails live here, outside any model: turn, action,
//! screenshot and wall-clock budgets, a failure brake, batch-halt
//! semantics, and cancellation checked between every step. The runner
//! owns no lock and no computer state: everything goes through
//! [`AgentComputer`], whose implementation runs each action through the
//! Core executor (rate limit → Pegoles Policy → control arbitration →
//! guest input).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use pegoles_core::CancellationToken;
use pegoles_protocol::{
    is_invisible_format, ActionOutcome, ActionResult, AgentEvent, AgentMessageKind, ComputerAction,
    TaskId, TaskStatus, MAX_AGENT_MESSAGE_CHARS,
};

use crate::planner::{
    CallOutcome, CallOutput, PlannedCall, Planner, PlannerError, PlannerTurn, Screenshot, Step,
};

/// The computer as the runner sees it. Implemented by the desktop app
/// over its single-owner app state; by fakes in tests.
pub trait AgentComputer: Send + Sync {
    /// Make the computer usable for agent input (created, running, guest
    /// runtime ready, input advertised). Bounded; honors `cancel`.
    fn prepare(&self, cancel: &CancellationToken) -> Result<(), String>;
    /// Capture the guest screen (policy-checked `ObserveScreen`).
    fn observe(&self, task: TaskId, cancel: &CancellationToken) -> Result<Screenshot, String>;
    /// Execute one action through the Core executor (policy, audit).
    fn act(&self, task: TaskId, action: ComputerAction, cancel: &CancellationToken)
        -> ActionResult;
    /// Record a task status transition (validated by Core).
    fn set_status(&self, task: TaskId, status: TaskStatus) -> Result<(), String>;
    /// Publish an event on the Core bus.
    fn publish(&self, event: AgentEvent);
}

/// Hard limits for one task run. Deterministic; never model-controlled.
#[derive(Clone, Debug)]
pub struct RunLimits {
    pub max_turns: u32,
    pub max_actions: u32,
    /// Wall-clock budget. It holds inside a turn and inside long steps
    /// too: at the deadline the run's cancellation token is tripped, so a
    /// wait, a long typed text or a model request in flight ends there.
    pub max_duration: Duration,
    /// Consecutive turns in which some call failed before giving up.
    pub max_failed_turns: u32,
    /// Calls executed per turn; the rest are answered "not executed".
    pub max_calls_per_turn: usize,
    /// Screenshots a planner may ask for in one run (the runner's own
    /// first and final frames are not counted).
    pub max_observations: u32,
    /// Screenshots per turn; later ones in the same turn are answered
    /// "not executed", so one batch never holds more frames than this.
    pub max_observations_per_turn: u32,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_turns: 80,
            max_actions: 400,
            max_duration: Duration::from_secs(30 * 60),
            max_failed_turns: 6,
            max_calls_per_turn: 50,
            max_observations: 160,
            max_observations_per_turn: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEnd {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunReport {
    pub end: RunEnd,
    pub summary: String,
    pub turns: u32,
    pub actions: u32,
}

const HALTED: &str = "Not executed: an earlier computer action in this turn failed.";

/// Watches a run's time budget from its own thread. At the deadline it
/// trips the run's cancellation token (the one Stop trips), so whatever
/// is in flight (a guest wait, a long typed text, a model request) ends
/// there instead of after it, and remembers that the clock did it.
struct Deadline {
    expired: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Deadline {
    fn start(
        started: Instant,
        budget: Duration,
        cancel: &CancellationToken,
    ) -> std::io::Result<Self> {
        let expired = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (flag, stop, token) = (expired.clone(), done.clone(), cancel.clone());
        let thread = std::thread::Builder::new()
            .name("pegoles-run-deadline".to_string())
            .spawn(move || loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let elapsed = started.elapsed();
                if elapsed >= budget {
                    flag.store(true, Ordering::SeqCst);
                    token.cancel();
                    return;
                }
                // Woken early when the run ends.
                std::thread::park_timeout(budget - elapsed);
            })?;
        Ok(Self {
            expired,
            done,
            thread: Some(thread),
        })
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// Run one task to its end. The caller owns the thread; this blocks.
/// When the time budget runs out the run trips `cancel` itself (see
/// [`RunLimits::max_duration`]) and ends as failed, not as stopped.
pub fn run_task(
    task: TaskId,
    objective: &str,
    planner: &mut dyn Planner,
    computer: &dyn AgentComputer,
    limits: &RunLimits,
    cancel: &CancellationToken,
) -> RunReport {
    let started = Instant::now();
    // Fail closed: without its watchdog a run could outlive its budget.
    let deadline = match Deadline::start(started, limits.max_duration, cancel) {
        Ok(d) => d,
        Err(e) => {
            return RunReport {
                end: RunEnd::Failed,
                summary: format!("task cannot start: {e}"),
                turns: 0,
                actions: 0,
            }
        }
    };
    let mut run = Run {
        task,
        computer,
        limits,
        cancel,
        started,
        expired: deadline.expired.clone(),
        turns: 0,
        actions: 0,
        observations: 0,
        turn_observations: 0,
        stop: None,
    };
    if let Err(e) = computer.set_status(task, TaskStatus::Running) {
        return run.report(RunEnd::Failed, format!("task cannot start: {e}"));
    }
    let end = run.drive(objective, planner);
    drop(deadline);
    let (end, summary) = end;
    let status = match end {
        RunEnd::Completed => TaskStatus::Completed,
        RunEnd::Failed => TaskStatus::Failed,
        RunEnd::Cancelled => TaskStatus::Cancelled,
    };
    run.message(
        if end == RunEnd::Completed {
            AgentMessageKind::Summary
        } else {
            AgentMessageKind::Error
        },
        &summary,
    );
    let _ = computer.set_status(task, status);
    run.report(end, summary)
}

struct Run<'a> {
    task: TaskId,
    computer: &'a dyn AgentComputer,
    limits: &'a RunLimits,
    cancel: &'a CancellationToken,
    started: Instant,
    /// Set by the deadline watchdog when it tripped `cancel`.
    expired: Arc<AtomicBool>,
    turns: u32,
    actions: u32,
    observations: u32,
    turn_observations: u32,
    /// A hard budget ran out inside a call: the run ends after the batch.
    stop: Option<String>,
}

impl Run<'_> {
    fn report(&self, end: RunEnd, summary: String) -> RunReport {
        RunReport {
            end,
            summary,
            turns: self.turns,
            actions: self.actions,
        }
    }

    fn message(&self, kind: AgentMessageKind, text: &str) {
        // Planner text is hostile whichever provider wrote it: invisible
        // formatting (bidi overrides, zero-width) and control characters
        // never reach the activity feed, where they could make a message
        // read differently than it is.
        let text: String = text
            .trim()
            .chars()
            .filter(|c| !is_invisible_format(*c))
            .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
            .take(MAX_AGENT_MESSAGE_CHARS)
            .collect();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.computer.publish(AgentEvent::AgentMessage {
            task_id: self.task,
            kind,
            text: text.to_string(),
            at: Utc::now(),
        });
    }

    fn out_of_time(&self) -> bool {
        self.expired.load(Ordering::SeqCst)
    }

    fn time_budget_spent(&self) -> (RunEnd, String) {
        (
            RunEnd::Failed,
            format!(
                "Stopped: the task exceeded its {} minute time budget.",
                self.limits.max_duration.as_secs() / 60
            ),
        )
    }

    fn cancelled(&self) -> Option<(RunEnd, String)> {
        if !self.cancel.is_cancelled() {
            return None;
        }
        Some(if self.out_of_time() {
            self.time_budget_spent()
        } else {
            (RunEnd::Cancelled, "Stopped by the user.".to_string())
        })
    }

    fn drive(&mut self, objective: &str, planner: &mut dyn Planner) -> (RunEnd, String) {
        if let Err(e) = self.computer.prepare(self.cancel) {
            return self
                .cancelled()
                .unwrap_or((RunEnd::Failed, format!("The computer is not ready: {e}")));
        }
        let first = match self.computer.observe(self.task, self.cancel) {
            Ok(shot) => shot,
            Err(e) => {
                return self
                    .cancelled()
                    .unwrap_or((RunEnd::Failed, format!("Could not see the screen: {e}")))
            }
        };
        if let Err(e) = planner.start(objective, &first) {
            return (RunEnd::Failed, planner_failure(&e));
        }
        let mut outcomes = Vec::new();
        let mut failed_turns = 0;
        loop {
            if let Some(end) = self.cancelled() {
                return end;
            }
            if self.started.elapsed() > self.limits.max_duration {
                return self.time_budget_spent();
            }
            if self.turns >= self.limits.max_turns {
                return (
                    RunEnd::Failed,
                    format!("Stopped: the task used all {} planning turns.", self.turns),
                );
            }
            self.turns += 1;
            let turn = match planner.next(std::mem::take(&mut outcomes), self.cancel) {
                Ok(turn) => turn,
                Err(PlannerError::Cancelled) => {
                    return self
                        .cancelled()
                        .unwrap_or((RunEnd::Cancelled, "Stopped by the user.".to_string()))
                }
                Err(e) => return (RunEnd::Failed, planner_failure(&e)),
            };
            match turn {
                PlannerTurn::Done { notes, summary } => {
                    for n in &notes {
                        self.message(AgentMessageKind::Progress, n);
                    }
                    // Evidence of the final state, recorded in the event
                    // stream (FrameObserved) before the task closes.
                    if let Err(e) = self.computer.observe(self.task, self.cancel) {
                        return self.cancelled().unwrap_or((
                            RunEnd::Failed,
                            format!("Could not capture the final screen: {e}"),
                        ));
                    }
                    return (RunEnd::Completed, summary);
                }
                PlannerTurn::Failed { notes, reason } => {
                    for n in &notes {
                        self.message(AgentMessageKind::Progress, n);
                    }
                    return (RunEnd::Failed, reason);
                }
                PlannerTurn::Calls { notes, calls } => {
                    for n in &notes {
                        self.message(AgentMessageKind::Progress, n);
                    }
                    if calls.is_empty() {
                        return (
                            RunEnd::Failed,
                            "The planner returned neither actions nor a result.".to_string(),
                        );
                    }
                    let batch = self.execute(calls);
                    if let Some(end) = self.cancelled() {
                        return end;
                    }
                    if let Some(reason) = self.stop.take() {
                        return (RunEnd::Failed, reason);
                    }
                    let any_failed = batch.iter().any(|o| o.result.is_err());
                    failed_turns = if any_failed { failed_turns + 1 } else { 0 };
                    if failed_turns >= self.limits.max_failed_turns {
                        return (
                            RunEnd::Failed,
                            format!(
                                "Stopped after {failed_turns} consecutive turns with failed actions."
                            ),
                        );
                    }
                    outcomes = batch;
                }
            }
        }
    }

    /// Run a batch in order; the first failure halts the rest.
    fn execute(&mut self, calls: Vec<PlannedCall>) -> Vec<CallOutcome> {
        let mut out = Vec::with_capacity(calls.len());
        let mut halted = false;
        self.turn_observations = 0;
        for (i, call) in calls.into_iter().enumerate() {
            if i >= self.limits.max_calls_per_turn {
                out.push(CallOutcome {
                    call_id: call.call_id,
                    result: Err(format!(
                        "Not executed: at most {} actions run per turn.",
                        self.limits.max_calls_per_turn
                    )),
                    skipped: true,
                });
                continue;
            }
            // The time budget holds inside a turn too.
            if !halted && self.started.elapsed() > self.limits.max_duration {
                halted = true;
            }
            if halted || self.cancel.is_cancelled() {
                out.push(CallOutcome {
                    call_id: call.call_id,
                    result: Err(HALTED.to_string()),
                    skipped: true,
                });
                continue;
            }
            let result = match call.steps {
                Err(reason) => Err(reason),
                Ok(steps) => self.run_steps(steps),
            };
            halted = result.is_err();
            out.push(CallOutcome {
                call_id: call.call_id,
                result,
                skipped: false,
            });
        }
        out
    }

    fn run_steps(&mut self, steps: Vec<Step>) -> Result<CallOutput, String> {
        let mut output = CallOutput::Text("OK".to_string());
        for step in steps {
            if self.cancel.is_cancelled() {
                return Err(if self.out_of_time() {
                    "Stopped: the task's time budget ran out."
                } else {
                    "Cancelled by the user."
                }
                .to_string());
            }
            // One call may carry many steps (a long text becomes several
            // actions): the time budget holds between them too.
            if self.started.elapsed() > self.limits.max_duration {
                return Err("Not executed: the task's time budget is exhausted.".to_string());
            }
            output = match step {
                Step::Reply(text) => CallOutput::Text(text),
                Step::Observe => {
                    if self.observations >= self.limits.max_observations {
                        self.stop = Some(format!(
                            "Stopped: the task used all {} screenshots.",
                            self.limits.max_observations
                        ));
                        return Err(format!(
                            "Screenshot budget exhausted ({} screenshots).",
                            self.limits.max_observations
                        ));
                    }
                    if self.turn_observations >= self.limits.max_observations_per_turn {
                        return Err(format!(
                            "Not executed: at most {} screenshots are taken per turn.",
                            self.limits.max_observations_per_turn
                        ));
                    }
                    self.observations += 1;
                    self.turn_observations += 1;
                    CallOutput::Image(self.computer.observe(self.task, self.cancel)?)
                }
                Step::Act(action) => {
                    if self.actions >= self.limits.max_actions {
                        self.stop = Some(format!(
                            "Stopped: the task used all {} actions.",
                            self.limits.max_actions
                        ));
                        return Err(format!(
                            "Action budget exhausted ({} actions).",
                            self.limits.max_actions
                        ));
                    }
                    self.actions += 1;
                    let result = self.computer.act(self.task, action, self.cancel);
                    action_output(&result)?
                }
            };
        }
        Ok(output)
    }
}

/// Map an executor result to what the planner sees. Policy denials and
/// approval requirements are explicit so a model can choose another path.
fn action_output(result: &ActionResult) -> Result<CallOutput, String> {
    match result.outcome {
        ActionOutcome::Executed => Ok(CallOutput::Text("OK".to_string())),
        ActionOutcome::Blocked => Err(format!("Blocked by Pegoles policy: {}", result.message)),
        ActionOutcome::NeedsApproval => Err(format!(
            "Requires user approval, which cannot be granted for this action: {}",
            result.message
        )),
        ActionOutcome::Interrupted => Err(format!("Interrupted: {}", result.message)),
        ActionOutcome::Failed => Err(format!("Failed: {}", result.message)),
    }
}

fn planner_failure(e: &PlannerError) -> String {
    match e {
        PlannerError::Auth(_) => {
            "The model rejected the API key. Check it in Settings.".to_string()
        }
        other => format!("The model could not continue: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripted::ScriptedPlanner;
    use pegoles_protocol::PointerButton;
    use std::sync::Mutex;

    /// Fake computer: records actions, can be told to block/fail some.
    #[derive(Default)]
    struct FakeComputer {
        acted: Mutex<Vec<ComputerAction>>,
        statuses: Mutex<Vec<TaskStatus>>,
        events: Mutex<Vec<AgentEvent>>,
        observed: Mutex<u32>,
        prepare_error: Option<String>,
        deny_clicks: bool,
        cancel_on_act: Option<CancellationToken>,
        /// Time each action takes.
        act_delay: Duration,
        /// Actions run until cancelled (a long text honoring Stop).
        act_until_cancelled: bool,
    }

    fn shot() -> Screenshot {
        Screenshot {
            png: vec![1, 2, 3],
            width: 100,
            height: 50,
        }
    }

    impl AgentComputer for FakeComputer {
        fn prepare(&self, _cancel: &CancellationToken) -> Result<(), String> {
            self.prepare_error.clone().map_or(Ok(()), Err)
        }
        fn observe(&self, _t: TaskId, _c: &CancellationToken) -> Result<Screenshot, String> {
            *self.observed.lock().unwrap() += 1;
            Ok(shot())
        }
        fn act(&self, _t: TaskId, action: ComputerAction, c: &CancellationToken) -> ActionResult {
            if let Some(token) = &self.cancel_on_act {
                token.cancel();
            }
            let blocked = self.deny_clicks && matches!(action, ComputerAction::Click { .. });
            self.acted.lock().unwrap().push(action);
            let now = Utc::now();
            std::thread::sleep(self.act_delay);
            if self.act_until_cancelled {
                let give_up = Instant::now() + Duration::from_secs(10);
                while !c.is_cancelled() && Instant::now() < give_up {
                    std::thread::sleep(Duration::from_millis(5));
                }
                return ActionResult::failed(
                    pegoles_protocol::ActionId::new(),
                    now,
                    ActionOutcome::Interrupted,
                    "interrupted: cancelled mid-action",
                );
            }
            if blocked {
                ActionResult::failed(
                    pegoles_protocol::ActionId::new(),
                    now,
                    ActionOutcome::Blocked,
                    "nope",
                )
            } else {
                ActionResult::executed(pegoles_protocol::ActionId::new(), now, "ok")
            }
        }
        fn set_status(&self, _t: TaskId, s: TaskStatus) -> Result<(), String> {
            self.statuses.lock().unwrap().push(s);
            Ok(())
        }
        fn publish(&self, e: AgentEvent) {
            self.events.lock().unwrap().push(e);
        }
    }

    fn click(id: &str) -> PlannedCall {
        PlannedCall {
            call_id: id.to_string(),
            label: "left_click".into(),
            steps: Ok(vec![Step::Act(ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            })]),
        }
    }

    fn key(id: &str) -> PlannedCall {
        PlannedCall {
            call_id: id.to_string(),
            label: "key".into(),
            steps: Ok(vec![Step::Act(ComputerAction::KeyPress {
                key: "Enter".into(),
            })]),
        }
    }

    #[test]
    fn completes_and_records_final_evidence() {
        let computer = FakeComputer::default();
        let mut planner =
            ScriptedPlanner::new(vec![vec![click("a"), key("b")]], |_| Ok("done".to_string()));
        let report = run_task(
            TaskId::new(),
            "do it",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Completed);
        assert_eq!(report.actions, 2);
        assert_eq!(
            *computer.statuses.lock().unwrap(),
            vec![TaskStatus::Running, TaskStatus::Completed]
        );
        // first observation + verify observation + final evidence frame
        assert!(*computer.observed.lock().unwrap() >= 2);
        assert!(computer.events.lock().unwrap().iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessage {
                kind: AgentMessageKind::Summary,
                ..
            }
        )));
    }

    #[test]
    fn policy_block_halts_the_batch_and_reports_back() {
        let computer = FakeComputer {
            deny_clicks: true,
            ..Default::default()
        };
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let mut planner = ScriptedPlanner::new(vec![vec![click("a"), key("b")]], |_| {
            Ok("unused".to_string())
        })
        .observing(move |outcomes| seen2.lock().unwrap().extend(outcomes.to_vec()));
        let _ = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        let seen = seen.lock().unwrap();
        assert!(matches!(&seen[0].result, Err(e) if e.contains("Blocked by Pegoles policy")));
        assert!(
            seen[1].skipped,
            "second call must be skipped after a failure"
        );
        // The key press never reached the computer.
        assert_eq!(computer.acted.lock().unwrap().len(), 1);
    }

    #[test]
    fn cancellation_mid_batch_stops_everything() {
        let cancel = CancellationToken::new();
        let computer = FakeComputer {
            cancel_on_act: Some(cancel.clone()),
            ..Default::default()
        };
        let mut planner = ScriptedPlanner::new(
            vec![vec![click("a"), key("b"), key("c")], vec![key("d")]],
            |_| Ok("unused".to_string()),
        );
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &cancel,
        );
        assert_eq!(report.end, RunEnd::Cancelled);
        assert_eq!(computer.acted.lock().unwrap().len(), 1);
        assert_eq!(
            computer.statuses.lock().unwrap().last(),
            Some(&TaskStatus::Cancelled)
        );
    }

    #[test]
    fn budgets_are_enforced_outside_the_model() {
        let computer = FakeComputer::default();
        let turns: Vec<Vec<PlannedCall>> = (0..50).map(|i| vec![key(&i.to_string())]).collect();
        let mut planner = ScriptedPlanner::new(turns, |_| Ok("unused".to_string()));
        let limits = RunLimits {
            max_turns: 5,
            ..Default::default()
        };
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Failed);
        assert_eq!(report.turns, 5);

        let computer = FakeComputer::default();
        let mut planner =
            ScriptedPlanner::new(vec![(0..10).map(|i| key(&i.to_string())).collect()], |_| {
                Ok("unused".to_string())
            });
        let limits = RunLimits {
            max_actions: 3,
            ..Default::default()
        };
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        assert_eq!(computer.acted.lock().unwrap().len(), 3);
        assert_eq!(report.actions, 3);
        // An exhausted action budget ends the run; the model does not get
        // more turns to keep asking.
        assert_eq!(report.end, RunEnd::Failed);
        assert!(
            report.summary.contains("all 3 actions"),
            "{}",
            report.summary
        );
        assert_eq!(report.turns, 1);
    }

    fn keys(id: &str, n: usize) -> PlannedCall {
        PlannedCall {
            call_id: id.to_string(),
            label: "type".into(),
            steps: Ok((0..n)
                .map(|_| Step::Act(ComputerAction::KeyPress { key: "a".into() }))
                .collect()),
        }
    }

    fn screenshot(id: &str) -> PlannedCall {
        PlannedCall {
            call_id: id.to_string(),
            label: "screenshot".into(),
            steps: Ok(vec![Step::Observe]),
        }
    }

    #[test]
    fn time_budget_holds_between_the_steps_of_one_call() {
        // One call packing 200 actions (a model's long text) against a
        // 150 ms budget: it stops mid-call, not after all 200.
        let computer = FakeComputer {
            act_delay: Duration::from_millis(5),
            ..Default::default()
        };
        let mut planner =
            ScriptedPlanner::new(vec![vec![keys("long", 200)]], |_| Ok("unused".into()));
        let limits = RunLimits {
            max_duration: Duration::from_millis(150),
            ..Default::default()
        };
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Failed);
        assert!(report.summary.contains("time budget"), "{}", report.summary);
        let acted = computer.acted.lock().unwrap().len();
        assert!(acted < 100, "{acted} actions ran past the time budget");
    }

    #[test]
    fn deadline_interrupts_a_step_in_flight() {
        // An action that only ends when cancelled (a long typed text): the
        // deadline trips the run's token, so it ends at the deadline and
        // the run reports the time budget, not a user stop.
        let computer = FakeComputer {
            act_until_cancelled: true,
            ..Default::default()
        };
        let mut planner = ScriptedPlanner::new(vec![vec![key("k")]], |_| Ok("unused".into()));
        let limits = RunLimits {
            max_duration: Duration::from_millis(200),
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let t0 = Instant::now();
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &cancel,
        );
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        assert_eq!(report.end, RunEnd::Failed);
        assert!(report.summary.contains("time budget"), "{}", report.summary);
        assert_eq!(
            computer.statuses.lock().unwrap().last(),
            Some(&TaskStatus::Failed)
        );
    }

    #[test]
    fn deadline_interrupts_a_model_request_in_flight() {
        /// A planner whose request never answers until cancelled.
        struct Hung;
        impl Planner for Hung {
            fn name(&self) -> String {
                "hung".into()
            }
            fn start(&mut self, _o: &str, _s: &Screenshot) -> Result<(), PlannerError> {
                Ok(())
            }
            fn next(
                &mut self,
                _o: Vec<CallOutcome>,
                cancel: &CancellationToken,
            ) -> Result<PlannerTurn, PlannerError> {
                let give_up = Instant::now() + Duration::from_secs(10);
                while !cancel.is_cancelled() && Instant::now() < give_up {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(PlannerError::Cancelled)
            }
        }
        let computer = FakeComputer::default();
        let limits = RunLimits {
            max_duration: Duration::from_millis(200),
            ..Default::default()
        };
        let t0 = Instant::now();
        let report = run_task(
            TaskId::new(),
            "x",
            &mut Hung,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        assert_eq!(report.end, RunEnd::Failed);
        assert!(report.summary.contains("time budget"), "{}", report.summary);
    }

    #[test]
    fn a_user_stop_is_still_reported_as_a_stop() {
        let cancel = CancellationToken::new();
        let computer = FakeComputer {
            cancel_on_act: Some(cancel.clone()),
            ..Default::default()
        };
        let mut planner = ScriptedPlanner::new(vec![vec![key("k")]], |_| Ok("unused".into()));
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &cancel,
        );
        assert_eq!(report.end, RunEnd::Cancelled);
        assert_eq!(report.summary, "Stopped by the user.");
    }

    #[test]
    fn screenshots_are_bounded_per_turn_and_per_run() {
        // 50 screenshots in one turn: only the per-turn cap is taken, the
        // rest are answered without touching the computer.
        let computer = FakeComputer::default();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let batch: Vec<PlannedCall> = (0..50).map(|i| screenshot(&i.to_string())).collect();
        let mut planner = ScriptedPlanner::new(vec![batch], |_| Ok("unused".into()))
            .observing(move |o| seen2.lock().unwrap().extend(o.to_vec()));
        let limits = RunLimits::default();
        run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        let seen = seen.lock().unwrap();
        let first_batch = &seen[..50];
        let images = first_batch
            .iter()
            .filter(|o| matches!(o.result, Ok(CallOutput::Image(_))))
            .count();
        assert_eq!(images as u32, limits.max_observations_per_turn);
        let refused = &first_batch[limits.max_observations_per_turn as usize];
        assert!(
            matches!(&refused.result, Err(e) if e.contains("per turn")),
            "{refused:?}"
        );
        // first frame + per-turn cap + verify + final evidence
        assert!(*computer.observed.lock().unwrap() <= limits.max_observations_per_turn + 3);

        // A screenshot loop across turns runs into the run budget and
        // ends the run.
        let computer = FakeComputer::default();
        let turns: Vec<Vec<PlannedCall>> =
            (0..60).map(|i| vec![screenshot(&i.to_string())]).collect();
        let mut planner = ScriptedPlanner::new(turns, |_| Ok("unused".into()));
        let limits = RunLimits {
            max_observations: 5,
            ..Default::default()
        };
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &limits,
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Failed);
        assert!(
            report.summary.contains("all 5 screenshots"),
            "{}",
            report.summary
        );
        // 5 budgeted + the runner's own first frame.
        assert_eq!(*computer.observed.lock().unwrap(), 6);
    }

    #[test]
    fn agent_messages_never_carry_invisible_or_control_characters() {
        struct Says(Option<PlannerTurn>);
        impl Planner for Says {
            fn name(&self) -> String {
                "says".into()
            }
            fn start(&mut self, _o: &str, _s: &Screenshot) -> Result<(), PlannerError> {
                Ok(())
            }
            fn next(
                &mut self,
                _o: Vec<CallOutcome>,
                _c: &CancellationToken,
            ) -> Result<PlannerTurn, PlannerError> {
                self.0
                    .take()
                    .ok_or(PlannerError::Protocol("done already".into()))
            }
        }
        let computer = FakeComputer::default();
        let mut planner = Says(Some(PlannerTurn::Done {
            notes: vec!["note \u{202E}txt.exe\u{200B}\u{1b}[31m".into()],
            summary: "All \u{2066}done\u{2069}\u{7}.\nSecond line".into(),
        }));
        run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        let texts: Vec<String> = computer
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AgentEvent::AgentMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        for t in &texts {
            assert!(
                !t.chars()
                    .any(|c| is_invisible_format(c) || (c.is_control() && c != '\n')),
                "{t:?}"
            );
        }
        assert_eq!(texts[1], "All done .\nSecond line");
    }

    #[test]
    fn repeated_failures_trip_the_brake() {
        let computer = FakeComputer {
            deny_clicks: true,
            ..Default::default()
        };
        let turns: Vec<Vec<PlannedCall>> = (0..20).map(|i| vec![click(&i.to_string())]).collect();
        let mut planner = ScriptedPlanner::new(turns, |_| Ok("unused".to_string()));
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Failed);
        assert!(report.summary.contains("consecutive"));
        assert_eq!(report.turns, 6);
    }

    #[test]
    fn unready_computer_fails_fast_with_reason() {
        let computer = FakeComputer {
            prepare_error: Some("image missing".into()),
            ..Default::default()
        };
        let mut planner = ScriptedPlanner::new(vec![], |_| Ok("x".into()));
        let report = run_task(
            TaskId::new(),
            "x",
            &mut planner,
            &computer,
            &RunLimits::default(),
            &CancellationToken::new(),
        );
        assert_eq!(report.end, RunEnd::Failed);
        assert!(report.summary.contains("image missing"));
        assert!(computer.acted.lock().unwrap().is_empty());
    }
}

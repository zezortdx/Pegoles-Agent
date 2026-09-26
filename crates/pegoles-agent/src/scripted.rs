//! Deterministic planner: a fixed sequence of turns, then a verification
//! of the final screen. Used by the real end-to-end test and for
//! scripted demos. It exercises exactly the same runner, executor and
//! policy path as a model planner; only the decision source differs.

use std::collections::VecDeque;

use crate::planner::{
    CallOutcome, CallOutput, PlannedCall, Planner, PlannerError, PlannerTurn, Screenshot, Step,
};

type Verify = Box<dyn FnMut(&Screenshot) -> Result<String, String> + Send>;
type Observer = Box<dyn FnMut(&[CallOutcome]) + Send>;

pub struct ScriptedPlanner {
    turns: VecDeque<Vec<PlannedCall>>,
    verify: Verify,
    observer: Option<Observer>,
    last_screen: Option<Screenshot>,
    verifying: bool,
}

impl ScriptedPlanner {
    /// `turns` run in order; afterwards the planner takes one screenshot
    /// and `verify` decides success (`Ok(summary)`) or failure.
    pub fn new(
        turns: Vec<Vec<PlannedCall>>,
        verify: impl FnMut(&Screenshot) -> Result<String, String> + Send + 'static,
    ) -> Self {
        Self {
            turns: turns.into(),
            verify: Box::new(verify),
            observer: None,
            last_screen: None,
            verifying: false,
        }
    }

    /// See every batch of outcomes (tests, harness logging).
    pub fn observing(mut self, observer: impl FnMut(&[CallOutcome]) + Send + 'static) -> Self {
        self.observer = Some(Box::new(observer));
        self
    }
}

impl Planner for ScriptedPlanner {
    fn name(&self) -> String {
        "script".to_string()
    }

    fn start(&mut self, _objective: &str, screen: &Screenshot) -> Result<(), PlannerError> {
        self.last_screen = Some(screen.clone());
        Ok(())
    }

    fn next(
        &mut self,
        outcomes: Vec<CallOutcome>,
        cancel: &pegoles_core::CancellationToken,
    ) -> Result<PlannerTurn, PlannerError> {
        if cancel.is_cancelled() {
            return Err(PlannerError::Cancelled);
        }
        if let Some(observer) = self.observer.as_mut() {
            observer(&outcomes);
        }
        for o in &outcomes {
            if let Ok(CallOutput::Image(shot)) = &o.result {
                self.last_screen = Some(shot.clone());
            }
        }
        if let Some(calls) = self.turns.pop_front() {
            return Ok(PlannerTurn::Calls {
                notes: Vec::new(),
                calls,
            });
        }
        if !self.verifying {
            self.verifying = true;
            return Ok(PlannerTurn::Calls {
                notes: vec!["Checking the result on screen.".to_string()],
                calls: vec![PlannedCall {
                    call_id: "verify".to_string(),
                    label: "screenshot".to_string(),
                    steps: Ok(vec![Step::Observe]),
                }],
            });
        }
        let screen = self
            .last_screen
            .as_ref()
            .ok_or_else(|| PlannerError::Protocol("no screen to verify".to_string()))?;
        Ok(match (self.verify)(screen) {
            Ok(summary) => PlannerTurn::Done {
                notes: Vec::new(),
                summary,
            },
            Err(reason) => PlannerTurn::Failed {
                notes: Vec::new(),
                reason,
            },
        })
    }
}

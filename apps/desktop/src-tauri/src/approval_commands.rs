//! Human authority for exact Core requests. Native prompts run outside Core locks.
use crate::commands::{lock_state, SharedState};
use crate::consent::Consent;
use pegoles_core::approvals::ApprovalDecision;
use pegoles_protocol::{ActionId, TaskStatus};

/// Validate before prompting and again under the grant lock. The decider never owns Core.
pub fn take_control(
    shared: &SharedState,
    consent: &dyn Consent,
    authority: &pegoles_core::human::HumanAuthority,
) -> Result<pegoles_core::ComputerView, String> {
    lock_state(shared)
        .registry
        .validate_human_authority(authority)?;
    if !consent.confirm_takeover()? {
        return Err("Human control was not confirmed".into());
    }
    let mut state = lock_state(shared);
    state.registry.take_snapshot_control(authority)?;
    Ok(state.registry.computer_view(state.preparing_image))
}

pub fn resolve(
    shared: &SharedState,
    consent: &dyn Consent,
    id: ActionId,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let pending = {
        let mut state = lock_state(shared);
        let pending = state.registry.approval(id)?;
        if state
            .tasks
            .get(&pending.task_id)
            .map_err(|_| "Unknown task")?
            .status
            != TaskStatus::WaitingForApproval
        {
            state.registry.discard_approval(id);
            return Err("Approval is no longer pending".into());
        }
        pending
    };
    let requested = decision;
    let decision = if decision == ApprovalDecision::ApproveOnce {
        match consent.confirm_agent_input(&pending.summary) {
            Ok(true) => ApprovalDecision::ApproveOnce,
            Ok(false) | Err(_) => ApprovalDecision::Deny,
        }
    } else {
        decision
    };
    let mut state = lock_state(shared);
    if state
        .tasks
        .get(&pending.task_id)
        .map_err(|_| "Unknown task")?
        .status
        != TaskStatus::WaitingForApproval
    {
        state.registry.discard_approval(id);
        return Err("Approval is no longer pending".into());
    }
    state.registry.resolve_approval(id, decision)?;
    if decision == ApprovalDecision::Deny && requested == ApprovalDecision::ApproveOnce {
        return Err("Action was denied".into());
    }
    Ok(())
}

pub fn change_policy(
    shared: &SharedState,
    consent: &dyn Consent,
    enabled: bool,
) -> Result<(), String> {
    if !consent.change_input_policy(enabled)? {
        return Err("Policy change was not confirmed".into());
    }
    lock_state(shared).registry.set_ask_before_input(enabled);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use pegoles_core::{BackendKind, CancellationToken, ComputerRegistry};
    use pegoles_protocol::{ComputerAction, TaskId};
    use std::sync::{Arc, Mutex};

    struct Decider {
        shared: SharedState,
        answer: bool,
        invalidate: bool,
    }
    impl Consent for Decider {
        fn confirm_takeover(&self) -> Result<bool, String> {
            let mut state = self
                .shared
                .try_lock()
                .expect("Core unlocked during native takeover prompt");
            if self.invalidate {
                state.registry.end_agent_session(&mut Vec::new());
            }
            Ok(self.answer)
        }

        fn allow_cloud_planner(&self, _: crate::agent::Provider) -> Result<bool, String> {
            panic!("no cloud")
        }
        fn api_key(&self, _: bool) -> Result<Option<String>, String> {
            panic!("no key")
        }
        fn confirm_agent_input(&self, summary: &str) -> Result<bool, String> {
            let mut state = self
                .shared
                .try_lock()
                .expect("Core must be unlocked during native prompt");
            assert_eq!(summary, "VM input: type");
            if self.invalidate {
                state.registry.cancel_agent_input("native prompt race");
            }
            Ok(self.answer)
        }
        fn change_input_policy(&self, _: bool) -> Result<bool, String> {
            let _state = self
                .shared
                .try_lock()
                .expect("Core must be unlocked during policy prompt");
            Ok(self.answer)
        }
    }
    struct TestDir(std::path::PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn rig() -> (SharedState, TestDir, TaskId, ActionId) {
        let dir = TestDir(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/review/agent-2.0")
                .join(format!("approval-test-{}", TaskId::new())),
        );
        std::fs::create_dir_all(&dir.0).unwrap();
        let mut state = AppState::with_registry(|bus| {
            ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.0.clone())
        });
        let mut mock = pegoles_computer::MockComputerBackend::new();
        mock.set_input_enabled(true);
        state.registry.create_default_on(Box::new(mock)).unwrap();
        state.registry.start().unwrap();
        state.registry.set_ask_before_input(true);
        state.registry.begin_agent_session(&mut Vec::new()).unwrap();
        let task = state.tasks.submit_task("native decision").unwrap().id;
        state.tasks.start_task(&task).unwrap();
        let result = state
            .registry
            .execute_action(
                task,
                ComputerAction::TypeText {
                    text: "ordinary text must not enter native prompt".into(),
                    sensitive: false,
                },
                false,
                &pegoles_policy::PolicyContext::default(),
                &CancellationToken::new(),
                false,
            )
            .0;
        state.tasks.wait_for_approval(&task).unwrap();
        (Arc::new(Mutex::new(state)), dir, task, result.action_id)
    }
    #[test]
    fn takeover_native_refusal_and_post_prompt_epoch_race_fail_closed() {
        for (answer, invalidate) in [(false, false), (true, true), (true, false)] {
            let (shared, _dir, _, _) = rig();
            let authority = {
                let mut state = lock_state(&shared);
                let meta = state
                    .registry
                    .capture_frame("takeover-test", &mut Vec::new())
                    .unwrap();
                pegoles_core::human::HumanAuthority {
                    computer_id: meta.computer_id,
                    frame_id: meta.frame_id,
                    width: meta.width_px,
                    height: meta.height_px,
                    generation: state.registry.control_generation(),
                }
            };
            let decider = Decider {
                shared: shared.clone(),
                answer,
                invalidate,
            };
            assert_eq!(
                take_control(&shared, &decider, &authority).is_ok(),
                answer && !invalidate
            );
            assert_eq!(
                lock_state(&shared).registry.control_owner()
                    == pegoles_protocol::ControlOwner::User,
                answer && !invalidate
            );
        }
    }
    #[test]
    fn native_refusal_never_grants_and_prompt_races_fail_closed() {
        for (answer, invalidate) in [(false, false), (true, true), (true, false)] {
            let (shared, _dir, task, id) = rig();
            let decider = Decider {
                shared: shared.clone(),
                answer,
                invalidate,
            };
            let decision = resolve(&shared, &decider, id, ApprovalDecision::ApproveOnce);
            assert_eq!(decision.is_ok(), answer && !invalidate);
            let mut state = lock_state(&shared);
            let result = state
                .registry
                .poll_approval(id, task, &CancellationToken::new());
            assert_eq!(
                matches!(
                    result,
                    pegoles_core::approvals::ApprovalProgress::Finished(
                        pegoles_protocol::ActionResult { success: true, .. }
                    )
                ),
                answer && !invalidate
            );
            assert!(state
                .registry
                .resolve_approval(id, ApprovalDecision::ApproveOnce)
                .is_err());
        }
    }
    #[test]
    fn terminal_tasks_and_unconfirmed_policy_cannot_gain_authority() {
        let (shared, _dir, task, id) = rig();
        let decider = Decider {
            shared: shared.clone(),
            answer: false,
            invalidate: false,
        };
        assert!(change_policy(&shared, &decider, false).is_err());
        assert!(lock_state(&shared).registry.ask_before_input());
        lock_state(&shared).tasks.cancel_task(&task).unwrap();
        assert!(resolve(&shared, &decider, id, ApprovalDecision::ApproveOnce).is_err());
        assert!(lock_state(&shared).registry.list_approvals().is_empty());
    }
}

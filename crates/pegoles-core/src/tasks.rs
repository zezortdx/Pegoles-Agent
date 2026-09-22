//! Basic task manager with verified transitions.
//!
//! Pending -> Running -> {Completed, Failed, Cancelled}
//! Running <-> WaitingForApproval
//! Pending -> Cancelled

use chrono::Utc;
use pegoles_protocol::{AgentEvent, AgentTask, TaskId, TaskStatus};
use std::collections::HashMap;

use crate::error::{CoreError, Result};
use crate::events::EventBus;

/// Longest task title accepted from the UI (Unicode scalar values).
pub const MAX_TASK_TITLE_CHARS: usize = 500;

/// Validate + normalize a user-supplied task title: trimmed, 1..=500
/// characters, single line, no control characters.
pub fn normalize_task_title(raw: &str) -> Result<String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(CoreError::InvalidTaskTitle("title is empty".to_string()));
    }
    let chars = title.chars().count();
    if chars > MAX_TASK_TITLE_CHARS {
        return Err(CoreError::InvalidTaskTitle(format!(
            "title is {chars} characters (max {MAX_TASK_TITLE_CHARS})"
        )));
    }
    if title.chars().any(char::is_control) {
        return Err(CoreError::InvalidTaskTitle(
            "title must be a single line without control characters".to_string(),
        ));
    }
    Ok(title.to_string())
}

#[derive(Debug, Default)]
pub struct TaskManager {
    tasks: HashMap<TaskId, AgentTask>,
    bus: EventBus,
}

impl TaskManager {
    pub fn new(bus: EventBus) -> Self {
        Self {
            tasks: HashMap::new(),
            bus,
        }
    }

    pub fn create_task(&mut self, title: impl Into<String>) -> AgentTask {
        let task = AgentTask::new(title);
        let id = task.id;
        let title_clone = task.title.clone();
        self.tasks.insert(id, task.clone());
        self.bus.publish(AgentEvent::TaskCreated {
            task_id: id,
            title: title_clone,
            at: Utc::now(),
        });
        task
    }

    /// Create a task from UI input (the Home CommandBar). Validated by
    /// `normalize_task_title`. Phase 4 has no model: nothing runs it, so
    /// it stays `Pending` until a future agent engine picks it up.
    pub fn submit_task(&mut self, raw_title: &str) -> Result<AgentTask> {
        let title = normalize_task_title(raw_title)?;
        Ok(self.create_task(title))
    }

    pub fn get(&self, id: &TaskId) -> Result<&AgentTask> {
        self.tasks
            .get(id)
            .ok_or_else(|| CoreError::TaskNotFound(id.to_string()))
    }

    pub fn list(&self) -> Vec<AgentTask> {
        let mut v: Vec<_> = self.tasks.values().cloned().collect();
        v.sort_by_key(|t| t.created_at);
        v
    }

    fn set_status(&mut self, id: &TaskId, to: TaskStatus) -> Result<AgentTask> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| CoreError::TaskNotFound(id.to_string()))?;
        let from = task.status;
        if !is_valid_transition(from, to) {
            return Err(CoreError::InvalidTaskTransition { from, to });
        }
        task.status = to;
        task.updated_at = Utc::now();
        let out = task.clone();
        self.bus.publish(AgentEvent::TaskStatusChanged {
            task_id: *id,
            from,
            to,
            at: Utc::now(),
        });
        Ok(out)
    }

    pub fn start_task(&mut self, id: &TaskId) -> Result<AgentTask> {
        self.set_status(id, TaskStatus::Running)
    }

    pub fn wait_for_approval(&mut self, id: &TaskId) -> Result<AgentTask> {
        self.set_status(id, TaskStatus::WaitingForApproval)
    }

    pub fn resume_from_approval(&mut self, id: &TaskId) -> Result<AgentTask> {
        self.set_status(id, TaskStatus::Running)
    }

    pub fn finish_task(&mut self, id: &TaskId) -> Result<AgentTask> {
        self.set_status(id, TaskStatus::Completed)
    }

    pub fn fail_task(&mut self, id: &TaskId) -> Result<AgentTask> {
        self.set_status(id, TaskStatus::Failed)
    }

    pub fn cancel_task(&mut self, id: &TaskId) -> Result<AgentTask> {
        let from = self.get(id)?.status;
        let to = TaskStatus::Cancelled;
        if !matches!(
            (from, to),
            (TaskStatus::Pending, TaskStatus::Cancelled)
                | (TaskStatus::Running, TaskStatus::Cancelled)
                | (TaskStatus::WaitingForApproval, TaskStatus::Cancelled)
        ) {
            return Err(CoreError::InvalidTaskTransition { from, to });
        }
        self.set_status(id, to)
    }
}

fn is_valid_transition(from: TaskStatus, to: TaskStatus) -> bool {
    matches!(
        (from, to),
        (TaskStatus::Pending, TaskStatus::Running)
            | (TaskStatus::Pending, TaskStatus::Cancelled)
            | (TaskStatus::Running, TaskStatus::WaitingForApproval)
            | (TaskStatus::Running, TaskStatus::Completed)
            | (TaskStatus::Running, TaskStatus::Failed)
            | (TaskStatus::Running, TaskStatus::Cancelled)
            | (TaskStatus::WaitingForApproval, TaskStatus::Running)
            | (TaskStatus::WaitingForApproval, TaskStatus::Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> TaskManager {
        TaskManager::new(EventBus::new())
    }

    #[test]
    fn create_starts_pending() {
        let mut m = mgr();
        let t = m.create_task("demo");
        assert_eq!(t.status, TaskStatus::Pending);
    }

    #[test]
    fn happy_path() {
        let mut m = mgr();
        let t = m.create_task("demo");
        let t = m.start_task(&t.id).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        let t = m.finish_task(&t.id).unwrap();
        assert_eq!(t.status, TaskStatus::Completed);
    }

    #[test]
    fn approval_cycle() {
        let mut m = mgr();
        let t = m.create_task("demo");
        m.start_task(&t.id).unwrap();
        m.wait_for_approval(&t.id).unwrap();
        let t = m.resume_from_approval(&t.id).unwrap();
        assert_eq!(t.status, TaskStatus::Running);
    }

    #[test]
    fn invalid_finish_from_pending() {
        let mut m = mgr();
        let t = m.create_task("demo");
        assert!(m.finish_task(&t.id).is_err());
    }

    #[test]
    fn cancel_from_pending_and_running() {
        let mut m = mgr();
        let a = m.create_task("a");
        assert_eq!(m.cancel_task(&a.id).unwrap().status, TaskStatus::Cancelled);
        let b = m.create_task("b");
        m.start_task(&b.id).unwrap();
        assert_eq!(m.cancel_task(&b.id).unwrap().status, TaskStatus::Cancelled);
    }

    #[test]
    fn cannot_cancel_completed() {
        let mut m = mgr();
        let t = m.create_task("x");
        m.start_task(&t.id).unwrap();
        m.finish_task(&t.id).unwrap();
        assert!(m.cancel_task(&t.id).is_err());
    }

    #[test]
    fn submit_task_trims_and_stays_pending() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let mut m = TaskManager::new(bus);
        let t = m.submit_task("  Summarize the logs  ").unwrap();
        assert_eq!(t.title, "Summarize the logs");
        assert_eq!(t.status, TaskStatus::Pending);
        match rx.try_recv().expect("task event") {
            AgentEvent::TaskCreated { task_id, title, .. } => {
                assert_eq!(task_id, t.id);
                assert_eq!(title, "Summarize the logs");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(m.list().len(), 1);
    }

    #[test]
    fn submit_task_rejects_bad_titles() {
        let mut m = mgr();
        for bad in ["", "   ", "\n\t", "line one\nline two", "bell\u{7}"] {
            assert!(
                matches!(m.submit_task(bad), Err(CoreError::InvalidTaskTitle(_))),
                "{bad:?}"
            );
        }
        let max = "é".repeat(MAX_TASK_TITLE_CHARS);
        assert!(
            m.submit_task(&max).is_ok(),
            "500 chars (not bytes) accepted"
        );
        let over = "x".repeat(MAX_TASK_TITLE_CHARS + 1);
        assert!(m.submit_task(&over).is_err());
        assert_eq!(m.list().len(), 1);
    }

    #[test]
    fn unknown_task_errors() {
        let mut m = mgr();
        assert!(m.start_task(&TaskId::new()).is_err());
    }
}

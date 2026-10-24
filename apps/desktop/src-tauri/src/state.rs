//! Single-owner app state: registry + task manager + shared event log.
//! All Tauri commands lock this Mutex briefly, OFF the main thread (see
//! `commands.rs`); no logic lives in React.
//!
//! Backend selection happens once here: real VM on macOS Apple Silicon,
//! Mock everywhere else (or when `PEGOLES_BACKEND=mock`).
//!
//! History: `history_rx` is a dedicated EventBus subscriber created before
//! anything can publish. Every command drains it into `event_log`, so
//! `list_events` sees exactly what the live `pegoles://event` stream
//! emitted — same events, same timestamps, no hand-written mirrors.

use pegoles_core::{default_backend_kind, ComputerRegistry, EventBus, TaskManager};
use pegoles_protocol::AgentEvent;
use std::collections::VecDeque;
use tokio::sync::broadcast::{self, error::TryRecvError};

#[derive(serde::Serialize)]
pub struct HistoryStatus {
    pub read_only: bool,
    pub previous_task_ids: Vec<pegoles_protocol::TaskId>,
}

pub const EVENT_LOG_CAP: usize = 200;

pub struct AppState {
    pub registry: ComputerRegistry,
    pub tasks: TaskManager,
    pub bus: EventBus,
    history_rx: broadcast::Receiver<AgentEvent>,
    pub event_log: VecDeque<AgentEvent>,
    history: pegoles_core::history::History,
    history_store: Option<pegoles_core::history::HistoryStore>,
    history_read_only: bool,
    previous_task_ids: Vec<pegoles_protocol::TaskId>,
    /// Events the history subscriber missed because more than the bus
    /// capacity were published between two commands (diagnostics; the
    /// live stream still delivered them).
    pub history_dropped: u64,
    pub preparing_image: bool,
    pub image_stage: Option<String>,
    pub image_downloaded: u64,
    pub image_total: u64,
    pub image_error: Option<String>,
    /// Cancels the running guest image installation (if any). Tripped
    /// without the app lock being needed by the installer thread.
    pub image_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Why the platform display adapter could not be installed, if so.
    /// (The adapter itself lives in `registry`, installed at startup by
    /// `native_display::install` on macOS.)
    pub display_error: Option<String>,
    /// Whether the chosen planner can run a task now (local model and
    /// runtime installed, or a cloud key present). Refreshed by the
    /// settings commands; no secret is held here.
    pub model_configured: bool,
    /// The chosen planner: `local` or `anthropic`.
    pub provider: &'static str,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_history_store(
            |bus| ComputerRegistry::with_backend_kind(bus.clone(), default_backend_kind()),
            pegoles_core::history::HistoryStore::new(
                pegoles_computer::pegoles_data_dir().join("history"),
            ),
        )
    }

    /// Build around a caller-made registry (tests: Mock + temp dirs).
    pub fn with_registry(make: impl FnOnce(&EventBus) -> ComputerRegistry) -> Self {
        let bus = EventBus::new();
        // Subscribe BEFORE anything can publish: history misses nothing.
        let history_rx = bus.subscribe();
        Self {
            registry: make(&bus),
            tasks: TaskManager::new(bus.clone()),
            bus,
            history_rx,
            event_log: VecDeque::new(),
            history: pegoles_core::history::History::default(),
            history_store: None,
            history_read_only: false,
            previous_task_ids: Vec::new(),
            history_dropped: 0,
            preparing_image: false,
            image_stage: None,
            image_downloaded: 0,
            image_total: 0,
            image_error: None,
            image_cancel: None,
            display_error: None,
            model_configured: false,
            provider: "local",
        }
    }

    /// Explicit test injection. `with_registry` alone never opens app data.
    pub fn with_history_store(
        make: impl FnOnce(&EventBus) -> ComputerRegistry,
        store: pegoles_core::history::HistoryStore,
    ) -> Self {
        let mut state = Self::with_registry(make);
        match store.read(chrono::Utc::now()) {
            Ok(history) => {
                if state.tasks.restore_history(&history.tasks()).is_ok() {
                    state.previous_task_ids = history.tasks().iter().map(|t| t.id).collect();
                    // Persist recovery and retention at startup as well: expired
                    // private data must leave disk even if no new task is made.
                    if std::fs::symlink_metadata(store.path()).is_ok()
                        && store.write(&history, chrono::Utc::now()).is_err()
                    {
                        state.history_read_only = true;
                    }
                    state.history = history;
                } else {
                    state.history_read_only = true;
                }
            }
            Err(_) => state.history_read_only = true,
        }
        state.history_store = Some(store);
        state
    }
    pub fn history_status(&self) -> HistoryStatus {
        HistoryStatus {
            read_only: self.history_read_only,
            previous_task_ids: self.previous_task_ids.clone(),
        }
    }
    pub fn task_history(
        &self,
        id: pegoles_protocol::TaskId,
    ) -> Option<pegoles_core::history::TaskHistory> {
        self.history.detail(id).cloned()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AgentEvent> {
        self.bus.subscribe()
    }

    /// Move everything published since the last call into history.
    /// Called under the app lock on a blocking worker; bounded disk I/O only.
    pub fn sync_history(&mut self) {
        let before = self.history.clone();
        self.history.update_tasks(&self.tasks.list());
        loop {
            match self.history_rx.try_recv() {
                Ok(event) => {
                    self.history.record(&event);
                    self.log(event);
                }
                Err(TryRecvError::Lagged(missed)) => {
                    self.history_dropped += missed;
                    self.history.block_outputs();
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
            }
        }
        if self.history.retain(chrono::Utc::now()).is_err() {
            self.history_read_only = true;
        }
        if self.history != before && !self.history_read_only {
            if let Some(store) = &self.history_store {
                if store.write(&self.history, chrono::Utc::now()).is_err() {
                    self.history_read_only = true;
                }
            }
        }
    }

    fn log(&mut self, event: AgentEvent) {
        if self.event_log.len() >= EVENT_LOG_CAP {
            self.event_log.pop_front();
        }
        self.event_log.push_back(event);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_core::BackendKind;
    use pegoles_protocol::ComputerConfig;

    fn mock_state(dir: &std::path::Path) -> AppState {
        let data = dir.to_path_buf();
        AppState::with_registry(|bus| {
            ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, data)
        })
    }

    #[test]
    fn startup_removes_expired_records_from_disk_not_just_the_view() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "pegoles-history-age-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = pegoles_core::history::HistoryStore::new(dir.clone());
        let mut history = pegoles_core::history::History::default();
        let mut task = pegoles_protocol::AgentTask::new("expired private objective");
        let old = chrono::Utc::now() - chrono::Duration::days(31);
        task.created_at = old;
        task.updated_at = old;
        history.update_tasks(&[task]);
        store.write(&history, old).unwrap();
        let state = AppState::with_history_store(
            |bus| {
                ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.join("computers"))
            },
            store,
        );
        assert!(state.tasks.list().is_empty());
        assert!(!std::fs::read_to_string(dir.join("tasks.json"))
            .unwrap()
            .contains("expired private objective"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persistent_restart_recovers_without_live_events_and_with_associated_result() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "pegoles-history-app-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let build = || {
            AppState::with_history_store(
                |bus| {
                    ComputerRegistry::with_dirs(
                        bus.clone(),
                        BackendKind::Mock,
                        dir.join("computers"),
                    )
                },
                pegoles_core::history::HistoryStore::new(dir.join("history")),
            )
        };
        let mut first = build();
        let a = first.tasks.submit_task("Make a report").unwrap();
        first.tasks.start_task(&a.id).unwrap();
        first.bus.publish(AgentEvent::AgentMessage {
            task_id: a.id,
            kind: pegoles_protocol::AgentMessageKind::Summary,
            text: "<script>inert result</script>".into(),
            at: chrono::Utc::now(),
        });
        first.sync_history();
        let mut second = build();
        assert_eq!(
            second.tasks.get(&a.id).unwrap().status,
            pegoles_protocol::TaskStatus::Cancelled
        );
        assert!(second.task_history(a.id).unwrap().artifact.is_some());
        assert!(second.history_status().previous_task_ids.contains(&a.id));
        second.sync_history();
        assert!(second.event_log.is_empty());
        assert!(!second.history_status().read_only);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_or_unwritable_store_is_read_only_and_never_overwritten() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "pegoles-history-bad-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tasks.json"), "original corrupt bytes").unwrap();
        let mut state = AppState::with_history_store(
            |bus| {
                ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.join("computers"))
            },
            pegoles_core::history::HistoryStore::new(dir.clone()),
        );
        assert!(state.history_status().read_only);
        state.tasks.submit_task("new task").unwrap();
        state.sync_history();
        assert_eq!(
            std::fs::read_to_string(dir.join("tasks.json")).unwrap(),
            "original corrupt bytes"
        );
        std::fs::remove_file(dir.join("tasks.json")).unwrap();
        let mut state = AppState::with_history_store(
            |bus| {
                ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.join("computers"))
            },
            pegoles_core::history::HistoryStore::new(dir.clone()),
        );
        std::fs::create_dir(dir.join("tasks.json")).unwrap();
        state.tasks.submit_task("new task").unwrap();
        state.sync_history();
        assert!(state.history_status().read_only);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lost_sensitive_events_discard_outputs_even_when_summary_survives() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "pegoles-history-lag-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut state = mock_state(&dir);
        let task = state.tasks.submit_task("Form task").unwrap();
        for _ in 0..300 {
            state.bus.publish(AgentEvent::TaskCreated {
                task_id: task.id,
                title: "discarded".into(),
                at: chrono::Utc::now(),
            });
        }
        state.bus.publish(AgentEvent::AgentMessage {
            task_id: task.id,
            kind: pegoles_protocol::AgentMessageKind::Summary,
            text: "potential sensitive echo".into(),
            at: chrono::Utc::now(),
        });
        state.sync_history();
        assert!(state.history_dropped > 0);
        assert!(state.task_history(task.id).unwrap().artifact.is_none());
    }

    #[test]
    fn history_mirrors_the_live_bus_exactly() {
        let tmp = std::env::temp_dir().join(format!("pegoles-desktop-{}", std::process::id()));
        let mut state = mock_state(&tmp);
        let mut live = state.subscribe();
        state.registry.create(ComputerConfig::default()).unwrap();
        state.registry.start().unwrap();
        state.tasks.submit_task("write a haiku").unwrap();
        state.sync_history();
        let mut streamed = Vec::new();
        while let Ok(ev) = live.try_recv() {
            streamed.push(ev);
        }
        assert!(!streamed.is_empty());
        assert_eq!(
            state.event_log.iter().cloned().collect::<Vec<_>>(),
            streamed,
            "history and live stream must carry the same events"
        );
        // Draining twice never duplicates.
        state.sync_history();
        assert_eq!(state.event_log.len(), streamed.len());
    }

    #[test]
    fn history_is_capped() {
        let tmp = std::env::temp_dir().join(format!("pegoles-desktop-cap-{}", std::process::id()));
        let mut state = mock_state(&tmp);
        for i in 0..(EVENT_LOG_CAP + 25) {
            state.tasks.submit_task(&format!("task {i}")).unwrap();
            if i % 50 == 0 {
                state.sync_history();
            }
        }
        state.sync_history();
        assert_eq!(state.event_log.len(), EVENT_LOG_CAP);
        assert_eq!(state.history_dropped, 0);
    }
}

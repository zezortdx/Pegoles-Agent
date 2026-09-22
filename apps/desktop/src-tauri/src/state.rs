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

pub const EVENT_LOG_CAP: usize = 200;

pub struct AppState {
    pub registry: ComputerRegistry,
    pub tasks: TaskManager,
    pub bus: EventBus,
    history_rx: broadcast::Receiver<AgentEvent>,
    pub event_log: VecDeque<AgentEvent>,
    /// Events the history subscriber missed because more than the bus
    /// capacity were published between two commands (diagnostics; the
    /// live stream still delivered them).
    pub history_dropped: u64,
    pub preparing_image: bool,
    pub image_stage: Option<String>,
    pub image_downloaded: u64,
    pub image_total: u64,
    pub image_error: Option<String>,
    /// Why the platform display adapter could not be installed, if so.
    /// (The adapter itself lives in `registry`, installed at startup by
    /// `native_display::install` on macOS.)
    pub display_error: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_registry(|bus| {
            ComputerRegistry::with_backend_kind(bus.clone(), default_backend_kind())
        })
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
            history_dropped: 0,
            preparing_image: false,
            image_stage: None,
            image_downloaded: 0,
            image_total: 0,
            image_error: None,
            display_error: None,
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AgentEvent> {
        self.bus.subscribe()
    }

    /// Move everything published since the last call into history.
    /// Never blocks.
    pub fn sync_history(&mut self) {
        loop {
            match self.history_rx.try_recv() {
                Ok(event) => self.log(event),
                Err(TryRecvError::Lagged(missed)) => self.history_dropped += missed,
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
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

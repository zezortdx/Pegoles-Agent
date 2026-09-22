//! Local event bus feeding UI / logs / audit / future mobile.
//! UI subscribes; it never drives core internals directly.

use pegoles_protocol::AgentEvent;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn publish(&self, event: AgentEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.tx.subscribe()
    }

    /// Drain-friendly snapshot for UIs that missed live events.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_protocol::TaskId;

    #[test]
    fn publish_reaches_subscriber() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let id = TaskId::new();
        bus.publish(AgentEvent::TaskCreated {
            task_id: id,
            title: "t".into(),
            at: chrono::Utc::now(),
        });
        let got = rx.try_recv().expect("event");
        match got {
            AgentEvent::TaskCreated { task_id, .. } => assert_eq!(task_id, id),
            other => panic!("unexpected {other:?}"),
        }
    }
}

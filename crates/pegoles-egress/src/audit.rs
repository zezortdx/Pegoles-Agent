//! Audit events: one per decision. Never headers, bodies, cookies or query
//! strings; the path is cut at `?`, stripped of control characters and
//! bounded.

use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::mpsc;

use crate::policy::{Decision, ModeKind};

/// Longest path kept in an event.
const MAX_PATH: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEvent {
    pub time: SystemTime,
    pub mode: ModeKind,
    /// Normalized host, or empty if the request named no valid host.
    pub host: String,
    /// Path without query or fragment (empty for CONNECT).
    pub path: String,
    pub decision: Decision,
    /// Body bytes relayed for this request (both directions).
    pub bytes: u64,
}

impl AuditEvent {
    pub fn new(mode: ModeKind, host: &str, path: &str, decision: Decision, bytes: u64) -> Self {
        AuditEvent {
            time: SystemTime::now(),
            mode,
            host: clean(host, 253),
            path: clean(path_without_query(path), MAX_PATH),
            decision,
            bytes,
        }
    }

    /// Stable reason code (`allowed` or a [`crate::policy::DenyReason`] code).
    pub fn reason_code(&self) -> &'static str {
        self.decision.reason_code()
    }
}

/// Where events go. Called from session tasks: it must not block.
pub type AuditSink = Arc<dyn Fn(AuditEvent) + Send + Sync>;

/// A sink that forwards to an unbounded channel.
pub fn channel() -> (AuditSink, mpsc::UnboundedReceiver<AuditEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink: AuditSink = Arc::new(move |e| {
        let _ = tx.send(e);
    });
    (sink, rx)
}

fn path_without_query(path: &str) -> &str {
    let end = path.find(['?', '#']).unwrap_or(path.len());
    &path[..end]
}

/// Printable ASCII only (anything else becomes `?`), bounded length.
fn clean(s: &str, max: usize) -> String {
    s.chars()
        .take(max)
        .map(|c| if c.is_ascii_graphic() { c } else { '?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::DenyReason;

    #[test]
    fn query_and_fragment_never_reach_the_event() {
        let e = AuditEvent::new(
            ModeKind::OpenWeb,
            "example.com",
            "/a/b?token=secret#frag",
            Decision::Allow,
            7,
        );
        assert_eq!(e.path, "/a/b");
        assert!(!format!("{e:?}").contains("secret"));
        assert_eq!(e.reason_code(), "allowed");
    }

    #[test]
    fn control_characters_and_length_are_bounded() {
        let e = AuditEvent::new(
            ModeKind::Allowlist,
            "a.example",
            &format!("/x\r\ny{}", "z".repeat(1000)),
            Decision::Deny(DenyReason::NotInAllowlist),
            0,
        );
        assert!(e.path.len() <= MAX_PATH);
        assert!(!e.path.contains(['\r', '\n']));
        assert_eq!(e.reason_code(), "mode_not_in_allowlist");
    }

    #[tokio::test]
    async fn channel_sink_delivers() {
        let (sink, mut rx) = channel();
        sink(AuditEvent::new(
            ModeKind::OpenWeb,
            "h.example",
            "/",
            Decision::Allow,
            1,
        ));
        let got = rx.recv().await.expect("event");
        assert_eq!(got.host, "h.example");
    }
}

//! Guest transport abstraction: the host side of one guest channel.
//!
//! Layering (strict):
//!
//! ```text
//! Guest Protocol v1 (GuestSession: handshake, heartbeat, timeout)
//!   ↓ consumes TransportEvent, emits frame payloads
//! GuestTransport (this trait: send / poll / close / liveness)
//!   ↓ implemented per hypervisor
//! macOS: VirtioSocketTransport (helper vsock link)
//! Windows (future): HyperVSocketTransport (AF_HYPERV)
//! tests: FakeGuestTransport (scripted)
//! ```
//!
//! The protocol never sees hypervisor types, and transports never parse
//! protocol messages: `Frame(String)` carries opaque JSONL. Pause/resume
//! may kill the channel on any platform — the session above reconnects,
//! so no transport promises survival across lifecycle transitions.

use crate::error::Result;

/// One observed transport fact. Protocol interpretation happens above.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportEvent {
    /// Channel established (fresh connect or reconnect).
    Connected,
    /// One complete, length-checked frame from the guest.
    Frame(String),
    /// Channel lost. `reason`: eof | frame_too_large | invalid_utf8 |
    /// kicked | helper_gone | not_implemented | ...
    Disconnected { reason: String },
}

/// Host side of one guest control channel.
pub trait GuestTransport: Send {
    /// Queue one JSONL frame for the guest (no trailing newline needed).
    fn send_frame(&mut self, payload: &str) -> Result<()>;
    /// Drain pending transport facts. Never blocks.
    fn poll_events(&mut self) -> Vec<TransportEvent>;
    /// Drop the channel (protocol violation, incompatibility).
    fn close(&mut self);
    /// Last known liveness (transport-level only, not handshake state).
    fn is_connected(&self) -> bool;
}

/// Scripted transport for driving `GuestSession` without any hypervisor.
/// Proves the protocol runs above ANY transport implementation.
#[derive(Debug, Default)]
pub struct FakeGuestTransport {
    /// Frames the session asked to send (observable by tests).
    pub sent: Vec<String>,
    inbound: std::collections::VecDeque<TransportEvent>,
    connected: bool,
    pub closed_count: u32,
}

impl FakeGuestTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test driver: inject a transport fact as if from the wire.
    pub fn inject(&mut self, event: TransportEvent) {
        match &event {
            TransportEvent::Connected => self.connected = true,
            TransportEvent::Disconnected { .. } => self.connected = false,
            TransportEvent::Frame(_) => {}
        }
        self.inbound.push_back(event);
    }
}

impl GuestTransport for FakeGuestTransport {
    fn send_frame(&mut self, payload: &str) -> Result<()> {
        if !self.connected {
            return Err(crate::error::ComputerError::GuestUnavailable(
                "fake transport disconnected".to_string(),
            ));
        }
        self.sent.push(payload.to_string());
        Ok(())
    }

    fn poll_events(&mut self) -> Vec<TransportEvent> {
        self.inbound.drain(..).collect()
    }

    fn close(&mut self) {
        self.closed_count += 1;
        self.connected = false;
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::GuestSession;
    use pegoles_guest_proto::encode_guest;

    /// The full handshake driven SOLELY through the transport trait: the
    /// session never sees which hypervisor (or fake) sits below.
    #[test]
    fn session_runs_above_any_transport() {
        use crate::guest::GuestObservation;
        use pegoles_protocol::GuestRuntimeState as G;

        let mut transport = FakeGuestTransport::new();
        let mut session = GuestSession::new();
        let now = std::time::Instant::now();
        session.on_vm_started(now);

        // Guest dials in.
        transport.inject(TransportEvent::Connected);
        // Helper: pump transport facts into the session, execute outbound
        // frames back through the transport.
        let pump = |transport: &mut FakeGuestTransport,
                    session: &mut GuestSession|
         -> Vec<crate::guest::GuestObservation> {
            let mut out = Vec::new();
            for ev in transport.poll_events() {
                let outcomes = match ev {
                    TransportEvent::Connected => session.on_connected(now),
                    TransportEvent::Frame(line) => session.on_frame(&line, now),
                    TransportEvent::Disconnected { reason } => session.on_disconnected(reason),
                };
                for o in outcomes {
                    let (action, observation) = o.split();
                    match action {
                        Some(crate::guest::Outbound::Send(frame)) => {
                            transport.send_frame(&frame).expect("connected");
                        }
                        Some(crate::guest::Outbound::Kick) => {
                            transport.close();
                        }
                        None => {}
                    }
                    if let Some(o) = observation {
                        out.push(o);
                    }
                }
            }
            out
        };

        let _ = pump(&mut transport, &mut session);
        assert_eq!(session.state(), G::Connecting);

        transport.inject(TransportEvent::Frame(
            r#"{"type":"guest_hello","protocol_version":1,"runtime_version":"0.1.0","os":"debian","os_version":"13","arch":"aarch64"}"#.to_string(),
        ));
        transport.inject(TransportEvent::Frame(encode_guest(
            &pegoles_guest_proto::GuestMessage::Ready,
        )));
        let obs = pump(&mut transport, &mut session);
        assert_eq!(session.state(), G::Ready);
        assert!(obs
            .iter()
            .any(|o| matches!(o, GuestObservation::BecameReady { .. })));
        // HostHello went out through the transport, not around it.
        assert!(transport.sent.iter().any(|f| f.contains("host_hello")));

        // Transport death surfaces as session Disconnect; reconnect works.
        transport.inject(TransportEvent::Disconnected {
            reason: "pause".to_string(),
        });
        let _ = pump(&mut transport, &mut session);
        assert_eq!(session.state(), G::Disconnected);
        transport.inject(TransportEvent::Connected);
        let _ = pump(&mut transport, &mut session);
        assert_eq!(session.state(), G::Connecting);
    }

    #[test]
    fn send_without_connection_fails_closed() {
        let mut t = FakeGuestTransport::new();
        assert!(t.send_frame("{}").is_err());
        assert!(!t.is_connected());
        t.inject(TransportEvent::Connected);
        assert!(t.send_frame("{}").is_ok());
        t.close();
        assert_eq!(t.closed_count, 1);
        assert!(t.send_frame("{}").is_err());
    }
}

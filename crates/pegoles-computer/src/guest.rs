//! Guest session: handshake, heartbeat, timeout, reconnect — pure logic.
//!
//! The session consumes transport facts (connected / frame / disconnected)
//! and produces outbound frames plus observations. It never touches IO:
//! `MacOSVirtualizationBackend` drives it from `poll_guest()` and the
//! blocking `guest_ping()` / `guest_info_request()` helpers.
//!
//! Readiness is rigorous: vsock connected + valid GuestHello + compatible
//! version + Ready seen, all inside `GUEST_READY_TIMEOUT` of VM start.
//! Anything else is Waiting/Connecting/Disconnected/Incompatible/Error.
//!
//! SECURITY: every inbound string is untrusted guest input. Fields are
//! length-bounded, unknown types ignored, malformed frames kill the
//! session (kick + Error), never the host.
//!
//! Graphical session (Phase 4): the guest reports its compositor status
//! (`GuestMessage::GraphicalSession`, pushed on change and answered to
//! `HostMessage::GetGraphicalSession`, which the host sends ONCE each
//! time the handshake reaches Ready — never polled). Reports are
//! untrusted: unbounded strings or absurd dimensions are protocol
//! violations. State, compositor, size and timing (ms since VM start)
//! are tracked here and reset on every VM start/stop; changes surface as
//! `SessionOutcome::GraphicalSessionChanged`.

use pegoles_guest_proto::{
    capabilities_bounded, diagnostics_bounded, encode_host, parse_guest_message,
    CapabilityDiagnostic, GraphicalSessionReport, GraphicalSessionStatus, GuestMessage,
    HostMessage, MessageError, SystemInfo, GUEST_PROTOCOL_VERSION, MAX_FRAME_CHUNKS,
    MAX_FRAME_CHUNK_B64, MAX_REQUEST_ID_BYTES,
};
use pegoles_protocol::{GraphicalSessionState, GuestRuntimeState};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long after VM start we wait for a completed handshake before
/// reporting Error. Justification: Debian boot takes ~20-40 s on this
/// hardware (Phase 2 smoke test); 60 s leaves margin without hanging the
/// UI forever. The VM keeps Running; only the guest state errors.
pub const GUEST_READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Heartbeat cadence (host Ping -> guest Pong).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// Missed-heartbeat tolerance before Disconnected (3 x 10 s = 30 s).
/// Distinct from VM state: the VM may run while the runtime is dead.
pub const HEARTBEAT_MISS_LIMIT: u32 = 3;
/// Per-field bound for untrusted guest strings.
pub const MAX_GUEST_FIELD: usize = 4096;
/// Bound for hello identity fields (os/arch/version strings).
pub const MAX_HELLO_FIELD: usize = 256;

/// Largest scanout dimension a guest may report (untrusted input bound;
/// well above `DisplayConfig::MAX_*` so real sizes always fit).
pub const MAX_REPORTED_DIMENSION_PX: u32 = 16_384;

fn bounded(s: &str, max: usize) -> bool {
    s.len() <= max
}

fn info_bounded(info: &SystemInfo) -> bool {
    [
        info.os.as_str(),
        info.os_version.as_str(),
        info.kernel.as_str(),
        info.arch.as_str(),
        info.hostname.as_str(),
        info.runtime_version.as_str(),
    ]
    .iter()
    .all(|s| bounded(s, MAX_GUEST_FIELD))
        && info.process_rss.len() <= pegoles_guest_proto::MAX_RSS_PROCESSES
        && info
            .process_rss
            .iter()
            .all(|p| bounded(&p.name, MAX_GUEST_FIELD))
}

fn report_dims_valid(report: &GraphicalSessionReport) -> bool {
    [report.width_px, report.height_px]
        .iter()
        .all(|d| d.is_none_or(|v| (1..=MAX_REPORTED_DIMENSION_PX).contains(&v)))
}

fn session_state_of(status: GraphicalSessionStatus) -> GraphicalSessionState {
    match status {
        GraphicalSessionStatus::Unavailable => GraphicalSessionState::Unavailable,
        GraphicalSessionStatus::Starting => GraphicalSessionState::Starting,
        GraphicalSessionStatus::Ready => GraphicalSessionState::Ready,
        GraphicalSessionStatus::Failed => GraphicalSessionState::Failed,
    }
}

/// Host-side view of the guest's graphical session (compositor), built
/// only from bounded guest reports. Reset on every VM start/stop; kept
/// across a transient vsock reconnect (the compositor does not die with
/// the control channel; the next Ready re-requests a fresh report).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicalSessionInfo {
    pub state: GraphicalSessionState,
    /// Whether the guest reported at all during this boot. Older guest
    /// runtimes never do: state stays `Unavailable` with `reported=false`.
    pub reported: bool,
    /// Compositor name as reported (informational, bounded).
    pub compositor: Option<String>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    /// Short guest diagnostic (failure reason), bounded.
    pub detail: Option<String>,
    /// ms from VM start to the report that entered the current state.
    pub since_ms: Option<u64>,
    /// ms from VM start to the FIRST Ready report of this boot.
    pub ready_in_ms: Option<u64>,
}

impl Default for GraphicalSessionInfo {
    fn default() -> Self {
        Self {
            state: GraphicalSessionState::Unavailable,
            reported: false,
            compositor: None,
            width_px: None,
            height_px: None,
            detail: None,
            since_ms: None,
            ready_in_ms: None,
        }
    }
}

/// Observations for the registry (events) and outbound frames for the
/// transport. Produced by `GuestSession::on_frame` / `tick`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionOutcome {
    /// One JSONL frame to send over vsock (via `guest_send`).
    SendFrame(String),
    /// Drop the vsock connection (via `guest_disconnect`).
    KickConnection,
    /// Transport came up (maps to GuestRuntimeConnected).
    Connected,
    /// A valid GuestHello was accepted (maps to GuestHandshakeCompleted).
    HandshakeCompleted { protocol_version: u32 },
    /// Full handshake done (maps to GuestRuntimeReady).
    BecameReady { ready_in_ms: u64 },
    /// State transition (maps to Disconnected/Incompatible/Error events).
    StateChanged {
        from: GuestRuntimeState,
        to: GuestRuntimeState,
        detail: Option<String>,
    },
    /// SystemInfo answer stored (no UI event; queried on demand).
    InfoReceived(Box<SystemInfo>),
    /// Heartbeat reply (no UI event; latency reported to smoke tests).
    PongReceived { nonce: u64, latency_ms: u64 },
    /// Graphical session changed state (or was reported for the first
    /// time this boot). Maps to GraphicalSessionReady / Failed events;
    /// Starting / Unavailable only move the derived viewport.
    GraphicalSessionChanged(GraphicalSessionChange),
    /// Guest answered `HostMessage::Input` (stateless routing; the
    /// executor matches `request_id` to its pending action).
    InputAckReceived {
        request_id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Guest began a frame transfer (`HostMessage::GetFrame` answer).
    FrameBeginReceived {
        request_id: String,
        width_px: u32,
        height_px: u32,
        total_chunks: u32,
    },
    /// One frame chunk (reassembled by the input executor, never logged).
    FrameChunkReceived {
        request_id: String,
        seq: u32,
        bytes: String,
    },
}

/// One graphical-session transition, with the facts of the report that
/// caused it. `at_ms` is measured from VM start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicalSessionChange {
    pub from: GraphicalSessionState,
    pub to: GraphicalSessionState,
    pub compositor: Option<String>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub detail: Option<String>,
    pub at_ms: u64,
}

/// What `poll_guest()` hands to Core: everything observable, with the
/// transport actions (SendFrame/KickConnection) already executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestObservation {
    Connected,
    HandshakeCompleted {
        protocol_version: u32,
    },
    BecameReady {
        ready_in_ms: u64,
    },
    StateChanged {
        from: GuestRuntimeState,
        to: GuestRuntimeState,
        detail: Option<String>,
    },
    InfoReceived(Box<SystemInfo>),
    PongReceived {
        nonce: u64,
        latency_ms: u64,
    },
    GraphicalSessionChanged(GraphicalSessionChange),
    InputAckReceived {
        request_id: String,
        ok: bool,
        error: Option<String>,
    },
    FrameBeginReceived {
        request_id: String,
        width_px: u32,
        height_px: u32,
        total_chunks: u32,
    },
    FrameChunkReceived {
        request_id: String,
        seq: u32,
        bytes: String,
    },
}

impl SessionOutcome {
    /// Split transport actions from observations. Returns the frame to
    /// send / whether to kick, plus the observation (if any).
    pub fn split(self) -> (Option<Outbound>, Option<GuestObservation>) {
        match self {
            SessionOutcome::SendFrame(f) => (Some(Outbound::Send(f)), None),
            SessionOutcome::KickConnection => (Some(Outbound::Kick), None),
            SessionOutcome::Connected => (None, Some(GuestObservation::Connected)),
            SessionOutcome::HandshakeCompleted { protocol_version } => (
                None,
                Some(GuestObservation::HandshakeCompleted { protocol_version }),
            ),
            SessionOutcome::BecameReady { ready_in_ms } => {
                (None, Some(GuestObservation::BecameReady { ready_in_ms }))
            }
            SessionOutcome::StateChanged { from, to, detail } => (
                None,
                Some(GuestObservation::StateChanged { from, to, detail }),
            ),
            SessionOutcome::InfoReceived(info) => {
                (None, Some(GuestObservation::InfoReceived(info)))
            }
            SessionOutcome::PongReceived { nonce, latency_ms } => (
                None,
                Some(GuestObservation::PongReceived { nonce, latency_ms }),
            ),
            SessionOutcome::GraphicalSessionChanged(change) => (
                None,
                Some(GuestObservation::GraphicalSessionChanged(change)),
            ),
            SessionOutcome::InputAckReceived {
                request_id,
                ok,
                error,
            } => (
                None,
                Some(GuestObservation::InputAckReceived {
                    request_id,
                    ok,
                    error,
                }),
            ),
            SessionOutcome::FrameBeginReceived {
                request_id,
                width_px,
                height_px,
                total_chunks,
            } => (
                None,
                Some(GuestObservation::FrameBeginReceived {
                    request_id,
                    width_px,
                    height_px,
                    total_chunks,
                }),
            ),
            SessionOutcome::FrameChunkReceived {
                request_id,
                seq,
                bytes,
            } => (
                None,
                Some(GuestObservation::FrameChunkReceived {
                    request_id,
                    seq,
                    bytes,
                }),
            ),
        }
    }
}

/// One transport action requested by the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outbound {
    Send(String),
    Kick,
}

#[derive(Debug)]
pub struct GuestSession {
    state: GuestRuntimeState,
    connected: bool,
    greeted: bool,
    guest_version: Option<u32>,
    /// Phase 5 capability advertisement from `GuestHello` (e.g. "input",
    /// "frame"). Empty for v0.1 guests. Reset on every VM start.
    guest_capabilities: Vec<String>,
    /// Phase 5.1 structured reasons for withheld capabilities.
    /// Reset on every VM start.
    capability_diagnostics: Vec<CapabilityDiagnostic>,
    info: Option<SystemInfo>,
    vm_started_at: Option<Instant>,
    /// Start of the current handshake window: VM start, then each new
    /// connection. A reconnect long after boot gets a fresh 60 s budget.
    handshake_started_at: Option<Instant>,
    ready_at: Option<Instant>,
    last_activity_at: Option<Instant>,
    last_ping_at: Option<Instant>,
    last_pong_at: Option<Instant>,
    ping_nonce: u64,
    pending_ping: Option<(u64, Instant)>,
    detail: Option<String>,
    graphical: GraphicalSessionInfo,
}

impl Default for GuestSession {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestSession {
    pub fn new() -> Self {
        Self {
            state: GuestRuntimeState::Unavailable,
            connected: false,
            greeted: false,
            guest_version: None,
            guest_capabilities: Vec::new(),
            capability_diagnostics: Vec::new(),
            info: None,
            vm_started_at: None,
            handshake_started_at: None,
            ready_at: None,
            last_activity_at: None,
            last_ping_at: None,
            last_pong_at: None,
            ping_nonce: 0,
            pending_ping: None,
            detail: None,
            graphical: GraphicalSessionInfo::default(),
        }
    }

    pub fn state(&self) -> GuestRuntimeState {
        self.state
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn guest_info(&self) -> Option<SystemInfo> {
        self.info.clone()
    }

    /// Advertised guest capabilities from the latest handshake (empty for
    /// v0.1 guests that predate advertisement).
    pub fn guest_capabilities(&self) -> &[String] {
        &self.guest_capabilities
    }

    /// True when the guest advertised a capability (input/frame).
    pub fn guest_supports(&self, capability: &str) -> bool {
        self.guest_capabilities.iter().any(|c| c == capability)
    }

    /// Structured reasons the guest withheld capabilities (empty when
    /// everything advertised, or for pre-5.1 guests).
    pub fn capability_diagnostics(&self) -> &[CapabilityDiagnostic] {
        &self.capability_diagnostics
    }

    /// Human-readable reason a capability is missing: guest diagnostic
    /// wins, else the honest default for silent (v0.1) guests.
    pub fn missing_capability_reason(&self, capability: &str) -> String {
        if let Some(d) = self
            .capability_diagnostics
            .iter()
            .find(|d| d.capability == capability)
        {
            return format!("guest reports {} unavailable: {}", capability, d.reason);
        }
        format!("guest runtime does not advertise '{capability}' support")
    }

    pub fn guest_version(&self) -> Option<u32> {
        self.guest_version
    }

    /// Last graphical-session facts reported by the guest this boot.
    pub fn graphical_session(&self) -> GraphicalSessionInfo {
        self.graphical.clone()
    }

    fn ms_since_vm_start(&self, now: Instant) -> u64 {
        self.vm_started_at
            .map(|start| now.saturating_duration_since(start).as_millis() as u64)
            .unwrap_or(0)
    }

    /// Apply one (already parsed) graphical-session report.
    fn on_graphical_report(
        &mut self,
        report: GraphicalSessionReport,
        now: Instant,
        out: &mut VecDeque<SessionOutcome>,
    ) {
        if !self.greeted {
            self.violation("graphical_session before hello".to_string(), out);
            return;
        }
        if !report.is_bounded() {
            self.violation("oversized graphical_session field".to_string(), out);
            return;
        }
        if !report_dims_valid(&report) {
            self.violation("graphical_session dimensions out of range".to_string(), out);
            return;
        }
        let at_ms = self.ms_since_vm_start(now);
        let from = self.graphical.state;
        let to = session_state_of(report.status);
        let first_report = !self.graphical.reported;
        let compositor = Some(report.compositor).filter(|c| !c.is_empty());
        self.graphical.reported = true;
        self.graphical.compositor = compositor.clone();
        self.graphical.width_px = report.width_px;
        self.graphical.height_px = report.height_px;
        self.graphical.detail = report.detail.clone();
        if from == to && !first_report {
            // Same state (e.g. an answer to our request after a push):
            // facts refreshed, no transition to report.
            return;
        }
        if from != to || self.graphical.since_ms.is_none() {
            self.graphical.since_ms = Some(at_ms);
        }
        if to == GraphicalSessionState::Ready && self.graphical.ready_in_ms.is_none() {
            self.graphical.ready_in_ms = Some(at_ms);
        }
        self.graphical.state = to;
        out.push_back(SessionOutcome::GraphicalSessionChanged(
            GraphicalSessionChange {
                from,
                to,
                compositor,
                width_px: report.width_px,
                height_px: report.height_px,
                detail: report.detail,
                at_ms,
            },
        ));
    }

    /// Milliseconds from VM start to Ready (real measured time for the UI).
    pub fn ready_duration_ms(&self) -> Option<u64> {
        match (self.vm_started_at, self.ready_at) {
            (Some(start), Some(ready)) => Some(ready.duration_since(start).as_millis() as u64),
            _ => None,
        }
    }

    fn set_state(
        &mut self,
        to: GuestRuntimeState,
        detail: Option<String>,
        out: &mut VecDeque<SessionOutcome>,
    ) {
        let from = self.state;
        if from == to {
            return;
        }
        self.state = to;
        self.detail = detail.clone();
        out.push_back(SessionOutcome::StateChanged { from, to, detail });
    }

    fn violation(&mut self, reason: String, out: &mut VecDeque<SessionOutcome>) {
        if self.connected {
            out.push_back(SessionOutcome::KickConnection);
            self.connected = false;
        }
        self.greeted = false;
        self.set_state(GuestRuntimeState::Error, Some(reason), out);
    }

    pub fn on_vm_started(&mut self, now: Instant) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        self.connected = false;
        self.greeted = false;
        self.guest_version = None;
        self.guest_capabilities = Vec::new();
        self.capability_diagnostics = Vec::new();
        self.info = None;
        self.ready_at = None;
        self.last_activity_at = None;
        self.last_ping_at = None;
        self.last_pong_at = None;
        self.pending_ping = None;
        self.graphical = GraphicalSessionInfo::default();
        self.vm_started_at = Some(now);
        self.handshake_started_at = Some(now);
        self.set_state(GuestRuntimeState::Waiting, None, &mut out);
        out.into()
    }

    pub fn on_vm_stopped(&mut self) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        *self = GuestSession::new();
        // Fresh session is Unavailable; report the transition if visible.
        let _ = &mut out;
        out.into()
    }

    pub fn on_connected(&mut self, now: Instant) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        self.connected = true;
        if self.state != GuestRuntimeState::Connecting && self.state != GuestRuntimeState::Ready {
            self.handshake_started_at = Some(now);
            self.set_state(GuestRuntimeState::Connecting, None, &mut out);
            out.push_back(SessionOutcome::Connected);
        }
        out.into()
    }

    pub fn on_disconnected(&mut self, reason: String) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        self.connected = false;
        self.greeted = false;
        self.pending_ping = None;
        match self.state {
            GuestRuntimeState::Ready | GuestRuntimeState::Connecting => {
                self.set_state(GuestRuntimeState::Disconnected, Some(reason), &mut out);
            }
            GuestRuntimeState::Waiting
            | GuestRuntimeState::Error
            | GuestRuntimeState::Incompatible => {
                // Transient drop before/after a failed handshake: stay,
                // but surface the reason for diagnostics.
                self.detail = Some(reason);
            }
            GuestRuntimeState::Unavailable | GuestRuntimeState::Disconnected => {}
        }
        out.into()
    }

    pub fn on_frame(&mut self, line: &str, now: Instant) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        let msg = match parse_guest_message(line) {
            Ok(m) => m,
            Err(MessageError::UnknownType) => {
                // Forward compatibility: ignore unknown guest messages.
                self.last_activity_at = Some(now);
                return out.into();
            }
            Err(_) => {
                self.violation("malformed guest frame".to_string(), &mut out);
                return out.into();
            }
        };
        self.last_activity_at = Some(now);
        match msg {
            GuestMessage::GuestHello {
                protocol_version,
                runtime_version,
                os,
                os_version,
                arch,
                capabilities,
                unavailable,
            } => {
                if ![
                    os.as_str(),
                    os_version.as_str(),
                    arch.as_str(),
                    runtime_version.as_str(),
                ]
                .iter()
                .all(|s| bounded(s, MAX_HELLO_FIELD))
                {
                    self.violation("oversized hello field".to_string(), &mut out);
                    return out.into();
                }
                if !capabilities_bounded(&capabilities) {
                    self.violation("oversized capability advertisement".to_string(), &mut out);
                    return out.into();
                }
                if !diagnostics_bounded(&unavailable) {
                    self.violation("oversized capability diagnostics".to_string(), &mut out);
                    return out.into();
                }
                if protocol_version != GUEST_PROTOCOL_VERSION {
                    self.guest_version = Some(protocol_version);
                    out.push_back(SessionOutcome::SendFrame(encode_host(
                        &HostMessage::Error {
                            code: "incompatible".to_string(),
                            message: format!("host speaks protocol {GUEST_PROTOCOL_VERSION}"),
                        },
                    )));
                    if self.connected {
                        out.push_back(SessionOutcome::KickConnection);
                        self.connected = false;
                    }
                    self.greeted = false;
                    self.set_state(
                        GuestRuntimeState::Incompatible,
                        Some(format!("guest protocol {protocol_version}")),
                        &mut out,
                    );
                    return out.into();
                }
                self.greeted = true;
                self.guest_version = Some(protocol_version);
                self.guest_capabilities = capabilities;
                self.capability_diagnostics = unavailable;
                out.push_back(SessionOutcome::SendFrame(encode_host(
                    &HostMessage::HostHello {
                        protocol_version: GUEST_PROTOCOL_VERSION,
                    },
                )));
                if self.state == GuestRuntimeState::Waiting
                    || self.state == GuestRuntimeState::Disconnected
                    || self.state == GuestRuntimeState::Error
                {
                    // Implicit connect (hello arrived before/without the
                    // transport event): accept it, don't stall.
                    self.connected = true;
                    self.set_state(GuestRuntimeState::Connecting, None, &mut out);
                    out.push_back(SessionOutcome::Connected);
                }
                out.push_back(SessionOutcome::HandshakeCompleted { protocol_version });
            }
            GuestMessage::Ready => {
                if !self.greeted {
                    self.violation("ready before hello".to_string(), &mut out);
                    return out.into();
                }
                if self.state != GuestRuntimeState::Ready {
                    self.ready_at = Some(now);
                    self.last_pong_at = Some(now);
                    let ms = self.ready_duration_ms().unwrap_or(0);
                    self.set_state(GuestRuntimeState::Ready, None, &mut out);
                    out.push_back(SessionOutcome::BecameReady { ready_in_ms: ms });
                    // Ask ONCE per handshake for the graphical session
                    // (the guest may have pushed a report already; older
                    // guests ignore the unknown type). Never polled.
                    out.push_back(SessionOutcome::SendFrame(encode_host(
                        &HostMessage::GetGraphicalSession,
                    )));
                }
            }
            GuestMessage::Pong { nonce } => {
                if self.pending_ping.map(|(n, _)| n) == Some(nonce) {
                    let sent = self.pending_ping.map(|(_, t)| t);
                    self.pending_ping = None;
                    self.last_pong_at = Some(now);
                    let latency = sent
                        .map(|t| now.duration_since(t).as_millis() as u64)
                        .unwrap_or(0);
                    out.push_back(SessionOutcome::PongReceived {
                        nonce,
                        latency_ms: latency,
                    });
                }
                // Stale/unsolicited pongs are ignored, not violations.
            }
            GuestMessage::SystemInfo(info) => {
                if !self.greeted {
                    self.violation("system_info before hello".to_string(), &mut out);
                    return out.into();
                }
                if !info_bounded(&info) {
                    self.violation("oversized system_info field".to_string(), &mut out);
                    return out.into();
                }
                self.info = Some(info.clone());
                out.push_back(SessionOutcome::InfoReceived(Box::new(info)));
            }
            GuestMessage::Error { code, message } => {
                let message = format!("guest reported {code}: {}", truncate(&message, 512));
                self.violation(message, &mut out);
            }
            GuestMessage::GraphicalSession(report) => {
                self.on_graphical_report(report, now, &mut out);
            }
            GuestMessage::InputAck {
                request_id,
                ok,
                error,
            } => {
                if !self.greeted {
                    self.violation("input_ack before hello".to_string(), &mut out);
                    return out.into();
                }
                if request_id.len() > MAX_REQUEST_ID_BYTES
                    || error.as_ref().is_some_and(|e| e.len() > MAX_GUEST_FIELD)
                {
                    self.violation("oversized input_ack field".to_string(), &mut out);
                    return out.into();
                }
                out.push_back(SessionOutcome::InputAckReceived {
                    request_id,
                    ok,
                    error,
                });
            }
            GuestMessage::FrameBegin {
                request_id,
                width_px,
                height_px,
                total_chunks,
            } => {
                if !self.greeted {
                    self.violation("frame_begin before hello".to_string(), &mut out);
                    return out.into();
                }
                if request_id.len() > MAX_REQUEST_ID_BYTES
                    || width_px == 0
                    || height_px == 0
                    || width_px > MAX_REPORTED_DIMENSION_PX
                    || height_px > MAX_REPORTED_DIMENSION_PX
                    || total_chunks == 0
                    || total_chunks > MAX_FRAME_CHUNKS
                {
                    self.violation("absurd frame_begin".to_string(), &mut out);
                    return out.into();
                }
                out.push_back(SessionOutcome::FrameBeginReceived {
                    request_id,
                    width_px,
                    height_px,
                    total_chunks,
                });
            }
            GuestMessage::FrameChunk {
                request_id,
                seq,
                bytes,
            } => {
                if !self.greeted {
                    self.violation("frame_chunk before hello".to_string(), &mut out);
                    return out.into();
                }
                if request_id.len() > MAX_REQUEST_ID_BYTES || bytes.len() > MAX_FRAME_CHUNK_B64 {
                    self.violation("oversized frame_chunk".to_string(), &mut out);
                    return out.into();
                }
                out.push_back(SessionOutcome::FrameChunkReceived {
                    request_id,
                    seq,
                    bytes,
                });
            }
        }
        out.into()
    }

    /// Explicit ping for `guest_ping()` round-trips (distinct from the
    /// automatic heartbeat in `tick`). Only meaningful when Ready.
    /// Returns the nonce plus the frame to send.
    pub fn manual_ping(&mut self, now: Instant) -> Option<(u64, String)> {
        if self.state != GuestRuntimeState::Ready {
            return None;
        }
        self.ping_nonce += 1;
        let nonce = self.ping_nonce;
        self.pending_ping = Some((nonce, now));
        self.last_ping_at = Some(now);
        Some((nonce, encode_host(&HostMessage::Ping { nonce })))
    }

    /// Periodic maintenance: handshake timeout, heartbeat pings, death
    /// detection. Call on every poll with a fresh `now`.
    pub fn tick(&mut self, now: Instant) -> Vec<SessionOutcome> {
        let mut out = VecDeque::new();
        match self.state {
            GuestRuntimeState::Waiting | GuestRuntimeState::Connecting => {
                if let Some(started) = self.handshake_started_at {
                    if now.duration_since(started) >= GUEST_READY_TIMEOUT {
                        if self.connected {
                            out.push_back(SessionOutcome::KickConnection);
                            self.connected = false;
                        }
                        self.greeted = false;
                        self.set_state(
                            GuestRuntimeState::Error,
                            Some("guest ready timeout (60 s)".to_string()),
                            &mut out,
                        );
                    }
                }
            }
            GuestRuntimeState::Ready => {
                let ping_due = self
                    .last_ping_at
                    .map(|t| now.duration_since(t) >= HEARTBEAT_INTERVAL)
                    .unwrap_or(true);
                if ping_due {
                    self.ping_nonce += 1;
                    let nonce = self.ping_nonce;
                    self.pending_ping = Some((nonce, now));
                    self.last_ping_at = Some(now);
                    out.push_back(SessionOutcome::SendFrame(encode_host(&HostMessage::Ping {
                        nonce,
                    })));
                }
                let silent = self
                    .last_activity_at
                    .map(|t| now.duration_since(t) >= HEARTBEAT_INTERVAL * HEARTBEAT_MISS_LIMIT)
                    .unwrap_or(false);
                if silent {
                    if self.connected {
                        out.push_back(SessionOutcome::KickConnection);
                        self.connected = false;
                    }
                    self.greeted = false;
                    self.pending_ping = None;
                    self.set_state(
                        GuestRuntimeState::Disconnected,
                        Some("heartbeat missed".to_string()),
                        &mut out,
                    );
                }
            }
            _ => {}
        }
        out.into()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_guest_proto::encode_guest;

    fn t0() -> Instant {
        Instant::now()
    }

    fn hello(v: u32) -> String {
        encode_guest(&GuestMessage::GuestHello {
            protocol_version: v,
            runtime_version: "0.1.0".into(),
            os: "debian".into(),
            os_version: "13".into(),
            arch: "aarch64".into(),
            capabilities: Vec::new(),
            unavailable: Vec::new(),
        })
    }

    fn hello_caps(v: u32, capabilities: Vec<String>) -> String {
        encode_guest(&GuestMessage::GuestHello {
            protocol_version: v,
            runtime_version: "0.1.0".into(),
            os: "debian".into(),
            os_version: "13".into(),
            arch: "aarch64".into(),
            capabilities,
            unavailable: Vec::new(),
        })
    }

    fn handshake(s: &mut GuestSession, now: Instant) -> Vec<SessionOutcome> {
        let mut all = s.on_connected(now);
        all.extend(s.on_frame(&hello(1), now));
        all.extend(s.on_frame(&encode_guest(&GuestMessage::Ready), now));
        all
    }

    #[test]
    fn full_handshake_to_ready() {
        let mut s = GuestSession::new();
        let now = t0();
        assert_eq!(s.state(), GuestRuntimeState::Unavailable);
        let out = s.on_vm_started(now);
        assert!(out.contains(&SessionOutcome::StateChanged {
            from: GuestRuntimeState::Unavailable,
            to: GuestRuntimeState::Waiting,
            detail: None,
        }));
        let out = s.on_connected(now);
        assert!(out.contains(&SessionOutcome::Connected));
        assert_eq!(s.state(), GuestRuntimeState::Connecting);

        let out = s.on_frame(&hello(1), now);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::SendFrame(f) if f.contains("host_hello"))));
        assert!(out.contains(&SessionOutcome::HandshakeCompleted {
            protocol_version: 1
        }));

        let out = s.on_frame(&encode_guest(&GuestMessage::Ready), now);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::BecameReady { .. })));
        assert_eq!(s.state(), GuestRuntimeState::Ready);
        assert_eq!(s.ready_duration_ms(), Some(0));
    }

    #[test]
    fn capabilities_advertised_and_reset_per_boot() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(
            &hello_caps(1, vec!["input".to_string(), "frame".to_string()]),
            now,
        );
        assert!(out.contains(&SessionOutcome::HandshakeCompleted {
            protocol_version: 1
        }));
        assert!(s.guest_supports("input"));
        assert!(s.guest_supports("frame"));
        assert!(!s.guest_supports("shell"));
        // v0.1 guests advertise nothing.
        let mut s2 = GuestSession::new();
        s2.on_vm_started(now);
        s2.on_connected(now);
        s2.on_frame(&hello(1), now);
        assert!(s2.guest_capabilities().is_empty());
        assert!(!s2.guest_supports("input"));
        // Reset on next boot.
        s.on_vm_started(now);
        assert!(s.guest_capabilities().is_empty());
        assert!(s.capability_diagnostics().is_empty());
    }

    #[test]
    fn withheld_capabilities_carry_structured_reasons() {
        use pegoles_guest_proto::CapabilityDiagnostic;
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let line = encode_guest(&GuestMessage::GuestHello {
            protocol_version: 1,
            runtime_version: "0.1.0".into(),
            os: "debian".into(),
            os_version: "13".into(),
            arch: "aarch64".into(),
            capabilities: vec!["frame".to_string()],
            unavailable: vec![CapabilityDiagnostic {
                capability: "input".to_string(),
                reason: "/dev/uinput: permission denied".to_string(),
            }],
        });
        let out = s.on_frame(&line, now);
        assert!(out.contains(&SessionOutcome::HandshakeCompleted {
            protocol_version: 1
        }));
        assert!(!s.guest_supports("input"));
        assert!(s.guest_supports("frame"));
        let diags = s.capability_diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].capability, "input");
        // Host refusal reasons surface the guest diagnostic verbatim.
        assert!(s
            .missing_capability_reason("input")
            .contains("permission denied"));
        // Silent (pre-5.1) guests get the honest default instead.
        let silent = GuestSession::new();
        assert_eq!(
            silent.missing_capability_reason("input"),
            "guest runtime does not advertise 'input' support"
        );
    }

    #[test]
    fn oversized_diagnostics_are_violation() {
        use pegoles_guest_proto::CapabilityDiagnostic;
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let line = encode_guest(&GuestMessage::GuestHello {
            protocol_version: 1,
            runtime_version: "0.1.0".into(),
            os: "debian".into(),
            os_version: "13".into(),
            arch: "aarch64".into(),
            capabilities: Vec::new(),
            unavailable: vec![CapabilityDiagnostic {
                capability: "input".into(),
                reason: "x".repeat(512),
            }],
        });
        let out = s.on_frame(&line, now);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn oversized_capabilities_are_violation() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(&hello_caps(1, vec!["x".repeat(64)]), now);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn input_ack_and_frame_chunks_route_statelessly() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        assert_eq!(s.state(), GuestRuntimeState::Ready);
        let ack = encode_guest(&GuestMessage::InputAck {
            request_id: "r1".into(),
            ok: true,
            error: None,
        });
        let (action, obs) = s.on_frame(&ack, now)[0].clone().split();
        assert!(action.is_none());
        assert_eq!(
            obs,
            Some(GuestObservation::InputAckReceived {
                request_id: "r1".into(),
                ok: true,
                error: None,
            })
        );
        let begin = encode_guest(&GuestMessage::FrameBegin {
            request_id: "f1".into(),
            width_px: 1440,
            height_px: 900,
            total_chunks: 2,
        });
        let out = s.on_frame(&begin, now);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::FrameBeginReceived { width_px: 1440, .. })));
        let chunk = encode_guest(&GuestMessage::FrameChunk {
            request_id: "f1".into(),
            seq: 0,
            bytes: "aGVsbG8=".into(),
        });
        let out = s.on_frame(&chunk, now);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::FrameChunkReceived { seq: 0, .. })));
        // Absurd frames are violations, not observations.
        let bad = encode_guest(&GuestMessage::FrameBegin {
            request_id: "f1".into(),
            width_px: 0,
            height_px: 900,
            total_chunks: 1,
        });
        let out = s.on_frame(&bad, now);
        assert!(out.contains(&SessionOutcome::KickConnection));
        // Acks before hello are violations (Error state; kick only
        // applies while connected).
        let mut fresh = GuestSession::new();
        fresh.on_vm_started(now);
        let out = fresh.on_frame(&ack, now);
        assert!(out.iter().any(|o| matches!(
            o,
            SessionOutcome::StateChanged {
                to: GuestRuntimeState::Error,
                ..
            }
        )));
    }

    #[test]
    fn ready_before_hello_is_violation() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(&encode_guest(&GuestMessage::Ready), now);
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn malformed_frame_kills_session_not_host() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame("{oops", now);
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn unknown_type_ignored() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(r#"{"type":"teleport","to":"mars"}"#, now);
        assert!(out.is_empty());
        assert_eq!(s.state(), GuestRuntimeState::Connecting);
    }

    #[test]
    fn version_mismatch_is_incompatible_not_silent() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(&hello(999), now);
        assert_eq!(s.state(), GuestRuntimeState::Incompatible);
        assert_eq!(s.guest_version(), Some(999));
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::SendFrame(f) if f.contains("incompatible"))));
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn waiting_times_out_after_60s() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        assert!(s.tick(now + Duration::from_secs(59)).is_empty());
        let out = s.tick(now + Duration::from_secs(61));
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.iter().any(|o| matches!(
            o,
            SessionOutcome::StateChanged {
                to: GuestRuntimeState::Error,
                ..
            }
        )));
    }

    #[test]
    fn reconnect_long_after_boot_gets_a_fresh_ready_budget() {
        // Found on hardware: a reconnect > 60 s after VM start timed out
        // instantly because the deadline was measured from boot.
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        s.on_disconnected("heartbeat missed".to_string());
        let later = now + Duration::from_secs(300);
        s.on_connected(later);
        assert!(s.tick(later + Duration::from_secs(1)).is_empty());
        assert_eq!(s.state(), GuestRuntimeState::Connecting);
        s.on_frame(&hello(1), later + Duration::from_secs(2));
        s.on_frame(
            &encode_guest(&GuestMessage::Ready),
            later + Duration::from_secs(2),
        );
        assert_eq!(s.state(), GuestRuntimeState::Ready);
        // A reconnect that never completes still times out.
        s.on_disconnected("gone".to_string());
        let again = later + Duration::from_secs(100);
        s.on_connected(again);
        s.tick(again + GUEST_READY_TIMEOUT + Duration::from_secs(1));
        assert_eq!(s.state(), GuestRuntimeState::Error);
    }

    #[test]
    fn heartbeat_ping_and_missed_death() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        // First tick sends a ping.
        let out = s.tick(now + HEARTBEAT_INTERVAL);
        let nonce = match out.iter().find_map(|o| match o {
            SessionOutcome::SendFrame(f) => serde_json::from_str::<serde_json::Value>(f).ok(),
            _ => None,
        }) {
            Some(v) => v["nonce"].as_u64().unwrap(),
            None => panic!("expected ping frame"),
        };
        // Pong keeps us alive.
        let later = now + HEARTBEAT_INTERVAL + Duration::from_secs(1);
        let out = s.on_frame(&encode_guest(&GuestMessage::Pong { nonce }), later);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::PongReceived { .. })));
        assert_eq!(s.state(), GuestRuntimeState::Ready);
        // Then silence: 30 s without frames -> Disconnected.
        let out =
            s.tick(later + HEARTBEAT_INTERVAL * HEARTBEAT_MISS_LIMIT + Duration::from_secs(1));
        assert_eq!(s.state(), GuestRuntimeState::Disconnected);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    #[test]
    fn stale_pong_ignored() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        let out = s.on_frame(&encode_guest(&GuestMessage::Pong { nonce: 12345 }), now);
        assert!(out.is_empty());
        assert_eq!(s.state(), GuestRuntimeState::Ready);
    }

    #[test]
    fn disconnect_then_reconnect_recovers() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        let out = s.on_disconnected("eof".to_string());
        assert_eq!(s.state(), GuestRuntimeState::Disconnected);
        assert!(out.iter().any(|o| matches!(
            o,
            SessionOutcome::StateChanged {
                to: GuestRuntimeState::Disconnected,
                ..
            }
        )));
        // Guest restarts: new connection + fresh handshake, no VM reboot.
        let later = now + Duration::from_secs(5);
        let out = s.on_connected(later);
        assert!(out.contains(&SessionOutcome::Connected));
        let out = s.on_frame(&hello(1), later);
        assert!(out.contains(&SessionOutcome::HandshakeCompleted {
            protocol_version: 1
        }));
        let out = s.on_frame(&encode_guest(&GuestMessage::Ready), later);
        assert!(out
            .iter()
            .any(|o| matches!(o, SessionOutcome::BecameReady { .. })));
        assert_eq!(s.state(), GuestRuntimeState::Ready);
    }

    #[test]
    fn vm_stop_resets_to_unavailable() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        s.on_vm_stopped();
        assert_eq!(s.state(), GuestRuntimeState::Unavailable);
        assert_eq!(s.guest_info(), None);
    }

    #[test]
    fn oversized_sysinfo_rejected() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        let big = "x".repeat(MAX_GUEST_FIELD + 1);
        let info = SystemInfo {
            os: "debian".into(),
            os_version: "13".into(),
            kernel: "k".into(),
            arch: "aarch64".into(),
            hostname: big,
            runtime_version: "0.1.0".into(),
            protocol_version: 1,
            uptime_s: None,
            cpu_count: None,
            mem_total_mb: None,
            mem_available_mb: None,
            cpu_jiffies: None,
            process_rss: Vec::new(),
        };
        let out = s.on_frame(&encode_guest(&GuestMessage::SystemInfo(info)), now);
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.contains(&SessionOutcome::KickConnection));
    }

    fn gfx(status: GraphicalSessionStatus, w: Option<u32>, h: Option<u32>) -> String {
        encode_guest(&GuestMessage::GraphicalSession(GraphicalSessionReport {
            status,
            compositor: "weston".into(),
            width_px: w,
            height_px: h,
            detail: None,
        }))
    }

    fn gfx_changes(out: &[SessionOutcome]) -> Vec<GraphicalSessionChange> {
        out.iter()
            .filter_map(|o| match o {
                SessionOutcome::GraphicalSessionChanged(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    fn get_gfx_requests(out: &[SessionOutcome]) -> usize {
        out.iter()
            .filter(|o| matches!(o, SessionOutcome::SendFrame(f) if f.contains("get_graphical_session")))
            .count()
    }

    #[test]
    fn ready_requests_graphical_session_exactly_once() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        let out = handshake(&mut s, now);
        assert_eq!(get_gfx_requests(&out), 1);
        // Heartbeats never re-request it (no polling).
        let out = s.tick(now + HEARTBEAT_INTERVAL);
        assert_eq!(get_gfx_requests(&out), 0);
        // A duplicate Ready while already Ready does not re-request.
        let out = s.on_frame(&encode_guest(&GuestMessage::Ready), now);
        assert_eq!(get_gfx_requests(&out), 0);
    }

    #[test]
    fn graphical_ready_report_tracked_with_timing() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        let at = now + Duration::from_millis(1500);
        let out = s.on_frame(
            &gfx(GraphicalSessionStatus::Starting, None, None),
            now + Duration::from_millis(900),
        );
        let changes = gfx_changes(&out);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, GraphicalSessionState::Unavailable);
        assert_eq!(changes[0].to, GraphicalSessionState::Starting);
        let out = s.on_frame(
            &gfx(GraphicalSessionStatus::Ready, Some(1440), Some(900)),
            at,
        );
        let changes = gfx_changes(&out);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to, GraphicalSessionState::Ready);
        assert_eq!(changes[0].at_ms, 1500);
        assert_eq!(changes[0].compositor.as_deref(), Some("weston"));
        let info = s.graphical_session();
        assert_eq!(info.state, GraphicalSessionState::Ready);
        assert!(info.reported);
        assert_eq!((info.width_px, info.height_px), (Some(1440), Some(900)));
        assert_eq!(info.ready_in_ms, Some(1500));
        assert_eq!(info.since_ms, Some(1500));
        // Session stays Ready: graphical reports never touch guest state.
        assert_eq!(s.state(), GuestRuntimeState::Ready);
    }

    #[test]
    fn same_state_report_is_not_a_transition() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        let ready = gfx(GraphicalSessionStatus::Ready, Some(1440), Some(900));
        assert_eq!(gfx_changes(&s.on_frame(&ready, now)).len(), 1);
        // Push + answer to our request: second identical report is silent.
        assert!(gfx_changes(&s.on_frame(&ready, now)).is_empty());
    }

    #[test]
    fn first_unavailable_report_is_observed() {
        // Unavailable -> Unavailable is not a state change, but the first
        // report of a boot is still a fact Core must see (reported=true).
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        let out = s.on_frame(&gfx(GraphicalSessionStatus::Unavailable, None, None), now);
        let changes = gfx_changes(&out);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to, GraphicalSessionState::Unavailable);
        assert!(s.graphical_session().reported);
    }

    #[test]
    fn failed_then_ready_keeps_first_ready_time() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        s.on_frame(
            &gfx(GraphicalSessionStatus::Ready, None, None),
            now + Duration::from_millis(100),
        );
        let failed = encode_guest(&GuestMessage::GraphicalSession(GraphicalSessionReport {
            status: GraphicalSessionStatus::Failed,
            compositor: "weston".into(),
            width_px: None,
            height_px: None,
            detail: Some("compositor exited".into()),
        }));
        let out = s.on_frame(&failed, now + Duration::from_millis(200));
        let changes = gfx_changes(&out);
        assert_eq!(changes[0].to, GraphicalSessionState::Failed);
        assert_eq!(changes[0].detail.as_deref(), Some("compositor exited"));
        s.on_frame(
            &gfx(GraphicalSessionStatus::Ready, None, None),
            now + Duration::from_millis(300),
        );
        let info = s.graphical_session();
        assert_eq!(info.state, GraphicalSessionState::Ready);
        assert_eq!(info.ready_in_ms, Some(100));
        assert_eq!(info.since_ms, Some(300));
    }

    #[test]
    fn graphical_report_before_hello_is_violation() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        s.on_connected(now);
        let out = s.on_frame(&gfx(GraphicalSessionStatus::Ready, None, None), now);
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.contains(&SessionOutcome::KickConnection));
        assert!(!s.graphical_session().reported);
    }

    #[test]
    fn oversized_graphical_report_is_violation() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        let big = encode_guest(&GuestMessage::GraphicalSession(GraphicalSessionReport {
            status: GraphicalSessionStatus::Ready,
            compositor: "w".repeat(pegoles_guest_proto::MAX_SESSION_FIELD_BYTES + 1),
            width_px: None,
            height_px: None,
            detail: None,
        }));
        let out = s.on_frame(&big, now);
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(out.contains(&SessionOutcome::KickConnection));
        assert_eq!(
            s.graphical_session().state,
            GraphicalSessionState::Unavailable
        );
    }

    #[test]
    fn absurd_graphical_dimensions_are_violation() {
        for (w, h) in [
            (Some(0), Some(900)),
            (Some(1440), Some(MAX_REPORTED_DIMENSION_PX + 1)),
        ] {
            let mut s = GuestSession::new();
            let now = t0();
            s.on_vm_started(now);
            handshake(&mut s, now);
            let out = s.on_frame(&gfx(GraphicalSessionStatus::Ready, w, h), now);
            assert_eq!(s.state(), GuestRuntimeState::Error, "{w:?}x{h:?}");
            assert!(out.contains(&SessionOutcome::KickConnection));
        }
    }

    #[test]
    fn graphical_session_resets_on_vm_start_and_stop() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        s.on_frame(&gfx(GraphicalSessionStatus::Ready, None, None), now);
        s.on_vm_stopped();
        assert_eq!(s.graphical_session(), GraphicalSessionInfo::default());
        s.on_vm_started(now);
        handshake(&mut s, now);
        s.on_frame(&gfx(GraphicalSessionStatus::Ready, None, None), now);
        s.on_vm_started(now + Duration::from_secs(1));
        assert_eq!(s.graphical_session(), GraphicalSessionInfo::default());
    }

    #[test]
    fn graphical_session_survives_reconnect_and_is_requested_again() {
        let mut s = GuestSession::new();
        let now = t0();
        s.on_vm_started(now);
        handshake(&mut s, now);
        s.on_frame(&gfx(GraphicalSessionStatus::Ready, None, None), now);
        s.on_disconnected("eof".to_string());
        assert_eq!(s.graphical_session().state, GraphicalSessionState::Ready);
        let out = handshake(&mut s, now + Duration::from_secs(2));
        assert_eq!(get_gfx_requests(&out), 1);
    }

    #[test]
    fn guest_error_frame_moves_to_error() {
        let mut s = GuestSession::new();
        let now = t0();
        handshake(&mut s, now);
        let out = s.on_frame(
            &encode_guest(&GuestMessage::Error {
                code: "boom".into(),
                message: "x".into(),
            }),
            now,
        );
        assert_eq!(s.state(), GuestRuntimeState::Error);
        assert!(!out.is_empty());
    }
}

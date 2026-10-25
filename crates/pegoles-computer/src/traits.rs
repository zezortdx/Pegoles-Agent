//! Backend trait. Implementors must never touch the host filesystem
//! or spawn processes on the host; guest operations only.
//!
//! Identity model: a Pegoles Computer is persistent (`ComputerId`), while
//! each `start()` creates an ephemeral [`ComputerInstance`] (new native
//! handle every time; on HCS the compute system is disposed on stop).
//! The Core tracks the computer; backends track their instances.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, GuestRuntimeState, SnapshotId};
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::platform::BackendCapabilities;

/// Ephemeral execution of a persistent computer: one `start()` produces
/// exactly one instance; `stop()` destroys it. Native handles (HCS compute
/// system, Vz runtime object generation, …) never leave the backend as
/// anything but this opaque id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerInstance {
    pub instance_id: String,
    pub computer_id: ComputerId,
    pub started_at: SystemTime,
}

impl ComputerInstance {
    pub fn new(computer_id: ComputerId) -> Self {
        Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            computer_id,
            started_at: SystemTime::now(),
        }
    }
}

pub trait ComputerBackend: Send + Sync {
    fn create(&mut self, config: ComputerConfig) -> Result<ComputerId>;
    fn start(&mut self) -> Result<ComputerState>;
    fn stop(&mut self) -> Result<ComputerState>;
    fn pause(&mut self) -> Result<ComputerState>;
    fn resume(&mut self) -> Result<ComputerState>;
    fn reset(&mut self) -> Result<ComputerState>;
    /// Delete the computer's persistent resources (disk, metadata) after
    /// stopping. The `ComputerId` must not be reused afterwards.
    fn destroy(&mut self) -> Result<()> {
        Err(crate::error::ComputerError::UnsupportedOperation(
            "destroy is not supported by this backend".to_string(),
        ))
    }
    fn snapshot(&mut self) -> Result<SnapshotId>;
    fn restore(&mut self, id: SnapshotId) -> Result<ComputerState>;
    fn state(&self) -> ComputerState;
    fn id(&self) -> Option<ComputerId>;
    /// The config this computer was created with, if any.
    fn config(&self) -> Option<ComputerConfig> {
        None
    }
    /// What this backend can actually do right now (reality, not roadmap).
    /// The future UI disables unsupported features from this.
    fn capabilities(&self) -> BackendCapabilities;
    /// The live ephemeral instance, if the computer is started. A fresh
    /// id is minted on every `start()`; `stop()` clears it.
    fn instance(&self) -> Option<ComputerInstance> {
        None
    }
    /// Guest serial console log (real backend only). Lets Core expose
    /// boot diagnostics without showing hundreds of lines in the main UI.
    fn serial_log_path(&self) -> Option<std::path::PathBuf> {
        None
    }
    // --- guest control plane (vsock). Defaults: no guest support. ---
    /// Handshake-derived guest state. Never conflated with VM state.
    fn guest_state(&self) -> GuestRuntimeState {
        GuestRuntimeState::Unavailable
    }
    /// Last SystemInfo reported by the guest, if any.
    fn guest_info(&self) -> Option<pegoles_guest_proto::SystemInfo> {
        None
    }
    /// Milliseconds from VM start to completed handshake, if measured.
    fn guest_ready_ms(&self) -> Option<u64> {
        None
    }
    /// Structured reasons the guest withheld input/capture capabilities
    /// (Phase 5.1 strict advertisement). Empty when all advertised or
    /// when the guest predates diagnostics.
    fn capability_diagnostics(&self) -> Vec<crate::CapabilityDiagnostic> {
        Vec::new()
    }
    /// Guest-reported graphical session (compositor) for this boot.
    /// Default: never reported (`Unavailable`) — Mock and headless
    /// backends have no graphical session.
    fn graphical_session(&self) -> crate::guest::GraphicalSessionInfo {
        crate::guest::GraphicalSessionInfo::default()
    }
    /// Drain pending guest transport events, run handshake/heartbeat
    /// maintenance, and return observations for Core (events).
    /// Never blocks; safe to call on every status poll.
    fn poll_guest(&mut self) -> Vec<crate::guest::GuestObservation> {
        Vec::new()
    }
    /// Blocking round-trip Ping -> Pong. Returns latency in ms.
    fn guest_ping(&mut self, _timeout: Duration) -> Result<u64> {
        Err(crate::error::ComputerError::UnsupportedOperation(
            "guest control plane not supported by this backend".to_string(),
        ))
    }
    /// Blocking GetSystemInfo round-trip.
    fn guest_info_request(
        &mut self,
        _timeout: Duration,
    ) -> Result<pegoles_guest_proto::SystemInfo> {
        Err(crate::error::ComputerError::UnsupportedOperation(
            "guest control plane not supported by this backend".to_string(),
        ))
    }
    // --- agent input plane (Phase 5, guest only). Defaults: no input. ---
    /// True when the guest advertised input support AND the session is
    /// Ready. The UI disables agent affordances while false.
    fn input_available(&self) -> bool {
        false
    }
    /// What this backend's input path can do right now.
    fn input_capabilities(&self) -> crate::input::InputCapabilities {
        crate::input::InputCapabilities {
            pointer: false,
            keyboard: false,
            screenshot: false,
            max_text_len: 0,
            kind: crate::input::InputBackendKind::Unavailable,
        }
    }
    /// Execute one backend primitive (guest pixels). Round-trips the
    /// guest over the control plane; honors `INPUT_ROUNDTRIP_MS`.
    fn input_execute(
        &mut self,
        _request_id: &str,
        _op: &crate::input::InputOp,
    ) -> crate::input::InputOutcome {
        crate::input::InputOutcome::failed("agent input not supported by this backend")
    }
    /// `input_execute` that stops waiting for the guest's ack once
    /// `cancelled()` turns true. Callers holding the app lock use it so
    /// Stop/Take Control never sit out a withheld ack. Default: the plain
    /// (bounded) call.
    fn input_execute_cancellable(
        &mut self,
        request_id: &str,
        op: &crate::input::InputOp,
        _cancelled: &dyn Fn() -> bool,
    ) -> crate::input::InputOutcome {
        self.input_execute(request_id, op)
    }
    /// Release everything the host believes is held (idempotent).
    fn input_release_all(&mut self) {}
    /// Guest pixels currently believed pressed (stuck-input audits).
    fn input_pressed(&self) -> crate::input::PressedState {
        crate::input::PressedState::default()
    }
    /// Blocking frame capture (guest framebuffer only, never the host
    /// screen). Returns raw RGBA + metadata.
    fn input_capture_frame(
        &mut self,
        _request_id: &str,
        _timeout: Duration,
    ) -> Result<crate::input::CapturedFrame> {
        Err(crate::error::ComputerError::UnsupportedOperation(
            "frame capture not supported by this backend".to_string(),
        ))
    }
    /// `input_capture_frame` that gives up once `cancelled()` turns true
    /// (a guest withholding chunks must not hold the app lock for the
    /// whole capture budget). Default: the plain (bounded) call.
    fn input_capture_frame_cancellable(
        &mut self,
        request_id: &str,
        timeout: Duration,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<crate::input::CapturedFrame> {
        self.input_capture_frame(request_id, timeout)
    }
    // --- egress stream (docs/EGRESS.md). Defaults: not supported. ---
    /// Open the one egress byte stream to the guest's forwarder. The
    /// backend creates the local endpoint first, asks the VM host helper
    /// to connect it to the guest (vsock port 4051), accepts exactly one
    /// connection (10 s) and returns it. The stream carries the mux
    /// protocol; everything on it is untrusted guest data. At most one
    /// stream per computer; a second call fails until `close_egress`.
    fn open_egress(&mut self) -> Result<crate::egress::EgressEndpoint> {
        Err(crate::error::ComputerError::UnsupportedOperation(
            "egress is not supported by this backend".to_string(),
        ))
    }
    /// First half of [`open_egress`](Self::open_egress) for callers that
    /// hold a lock: validate, create the local endpoint and send the
    /// helper's command, without waiting for anything (short, bounded).
    /// The returned handle waits for the helper's answer and the guest's
    /// connection and needs no access to the backend, so the caller can
    /// wait with its lock released. Default: opens synchronously.
    fn begin_open_egress(&mut self) -> Result<Box<dyn crate::egress::PendingEgressOpen>> {
        Ok(Box::new(crate::egress::ReadyEgress(self.open_egress()?)))
    }
    /// Tell the helper to close the egress stream (idempotent). Also
    /// happens implicitly on stop, destroy and reset.
    fn close_egress(&mut self) -> Result<()> {
        Ok(())
    }
}

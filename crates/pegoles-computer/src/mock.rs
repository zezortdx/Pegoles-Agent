//! Mock backend: deterministic in-memory state machine for dev/tests.
//!
//! Create -> Stopped ->(start) Running ->(pause) Paused ->(resume) Running
//! ->(stop) Stopped. `reset()` returns to Stopped from anywhere.
//! Invalid transitions return `ComputerError::InvalidTransition`.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, SnapshotId};
use std::collections::HashMap;

use crate::error::{ComputerError, Result};
use crate::input::{InputBackendKind, InputCapabilities, InputOp, InputOutcome, PressedState};
use crate::platform::{BackendCapabilities, DiskFormat, GuestArchitecture};
use crate::traits::{ComputerBackend, ComputerInstance};

#[derive(Debug)]
pub struct MockComputerBackend {
    id: Option<ComputerId>,
    state: Option<ComputerState>,
    config: Option<ComputerConfig>,
    snapshots: HashMap<u64, ComputerState>,
    next_snapshot: u64,
    instance: Option<ComputerInstance>,
    /// Agent input switch (default off: the mock has no guest until a
    /// test enables it, mirroring capability-gated hardware).
    input_enabled: bool,
    input_log: Vec<InputOp>,
    pressed: PressedState,
    /// The egress stream is open (mirrors the real backend's rule: one
    /// stream per computer, closed by stop/reset).
    egress_open: bool,
    /// The "guest" end of the egress stream until a test takes it.
    #[cfg(unix)]
    egress_peer: EgressPeerHandle,
    /// How long an egress open takes to complete (a slow helper).
    egress_open_delay: std::time::Duration,
}

/// Shared slot for the guest end of the mock egress stream: a test keeps a
/// handle before handing the backend to Core, which owns it afterwards.
#[cfg(unix)]
#[derive(Clone, Debug, Default)]
pub struct EgressPeerHandle(
    std::sync::Arc<std::sync::Mutex<Option<std::os::unix::net::UnixStream>>>,
);

#[cfg(unix)]
impl EgressPeerHandle {
    /// The far end, once (`None` until `open_egress`, or after a close).
    pub fn take(&self) -> Option<std::os::unix::net::UnixStream> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    fn put(&self, peer: Option<std::os::unix::net::UnixStream>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = peer;
    }
}

impl Default for MockComputerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockComputerBackend {
    pub fn new() -> Self {
        Self {
            id: None,
            state: None,
            config: None,
            snapshots: HashMap::new(),
            next_snapshot: 1,
            instance: None,
            input_enabled: false,
            input_log: Vec::new(),
            pressed: PressedState::default(),
            egress_open: false,
            #[cfg(unix)]
            egress_peer: EgressPeerHandle::default(),
            egress_open_delay: std::time::Duration::ZERO,
        }
    }

    /// Makes an egress open take `delay` to complete, like a slow helper:
    /// `begin_open_egress` returns at once, waiting takes the delay.
    pub fn set_egress_open_delay(&mut self, delay: std::time::Duration) {
        self.egress_open_delay = delay;
    }

    /// A handle to the guest end of the egress stream that stays valid
    /// after the backend is boxed into Core.
    #[cfg(unix)]
    pub fn egress_peer_handle(&self) -> EgressPeerHandle {
        self.egress_peer.clone()
    }

    /// The far end of the open egress stream (what the guest's forwarder
    /// would hold), once. `None` until `open_egress`. Dropping it is the
    /// guest closing the stream.
    #[cfg(unix)]
    pub fn take_egress_peer(&mut self) -> Option<std::os::unix::net::UnixStream> {
        self.egress_peer.take()
    }

    /// Enable the mock input plane for executor/runner tests.
    pub fn set_input_enabled(&mut self, enabled: bool) {
        self.input_enabled = enabled;
    }

    /// Recorded input primitives, in dispatch order.
    pub fn input_log(&self) -> &[InputOp] {
        &self.input_log
    }

    fn close_egress_now(&mut self) {
        self.egress_open = false;
        #[cfg(unix)]
        self.egress_peer.put(None);
    }

    fn require_created(&self) -> Result<(ComputerState, ComputerId)> {
        match (self.state, self.id) {
            (Some(s), Some(id)) => Ok((s, id)),
            _ => Err(ComputerError::NotCreated),
        }
    }

    fn transition(&mut self, to: ComputerState) -> Result<ComputerState> {
        let (from, _) = self.require_created()?;
        let valid = matches!(
            (from, to),
            (ComputerState::Stopped, ComputerState::Running)
                | (ComputerState::Stopped, ComputerState::Stopped)
                | (ComputerState::Running, ComputerState::Paused)
                | (ComputerState::Running, ComputerState::Stopped)
                | (ComputerState::Paused, ComputerState::Running)
                | (ComputerState::Paused, ComputerState::Stopped)
        );
        if !valid {
            return Err(ComputerError::InvalidTransition { from, to });
        }
        self.state = Some(to);
        if to == ComputerState::Stopped {
            self.close_egress_now();
        }
        Ok(to)
    }
}

impl ComputerBackend for MockComputerBackend {
    fn create(&mut self, config: ComputerConfig) -> Result<ComputerId> {
        if let Some(id) = self.id {
            return Err(ComputerError::AlreadyCreated(id));
        }
        let id = ComputerId::new();
        self.id = Some(id);
        self.config = Some(config);
        self.state = Some(ComputerState::Stopped);
        Ok(id)
    }

    fn start(&mut self) -> Result<ComputerState> {
        let (from, id) = self.require_created()?;
        match from {
            ComputerState::Stopped => {
                let state = self.transition(ComputerState::Running)?;
                // Fresh ephemeral instance per start; the computer id stays.
                self.instance = Some(ComputerInstance::new(id));
                Ok(state)
            }
            _ => Err(ComputerError::InvalidTransition {
                from,
                to: ComputerState::Running,
            }),
        }
    }

    fn stop(&mut self) -> Result<ComputerState> {
        self.require_created()?;
        let state = self.transition(ComputerState::Stopped)?;
        self.instance = None;
        Ok(state)
    }

    fn pause(&mut self) -> Result<ComputerState> {
        let (from, _) = self.require_created()?;
        match from {
            ComputerState::Running => self.transition(ComputerState::Paused),
            _ => Err(ComputerError::InvalidTransition {
                from,
                to: ComputerState::Paused,
            }),
        }
    }

    fn resume(&mut self) -> Result<ComputerState> {
        let (from, _) = self.require_created()?;
        match from {
            ComputerState::Paused => self.transition(ComputerState::Running),
            _ => Err(ComputerError::InvalidTransition {
                from,
                to: ComputerState::Running,
            }),
        }
    }

    fn reset(&mut self) -> Result<ComputerState> {
        self.require_created()?;
        self.state = Some(ComputerState::Stopped);
        self.instance = None;
        self.close_egress_now();
        Ok(ComputerState::Stopped)
    }

    fn snapshot(&mut self) -> Result<SnapshotId> {
        let (state, _) = self.require_created()?;
        let id = SnapshotId(self.next_snapshot);
        self.next_snapshot += 1;
        self.snapshots.insert(id.0, state);
        Ok(id)
    }

    fn restore(&mut self, id: SnapshotId) -> Result<ComputerState> {
        self.require_created()?;
        match self.snapshots.get(&id.0).copied() {
            Some(state) => {
                self.state = Some(state);
                Ok(state)
            }
            None => Err(ComputerError::UnknownSnapshot(id)),
        }
    }

    fn state(&self) -> ComputerState {
        // No computer yet: report Stopped so UI shows "Not created"
        // via `id().is_none()` instead of inventing a new state.
        self.state.unwrap_or(ComputerState::Stopped)
    }

    fn id(&self) -> Option<ComputerId> {
        self.id
    }

    fn config(&self) -> Option<ComputerConfig> {
        self.config.clone()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            pause: true,
            resume: true,
            snapshot: true,
            graphical_display: false,
            vsock: false,
            dynamic_memory: false,
            guest_arch: GuestArchitecture::Arm64,
            disk_formats: vec![DiskFormat::Raw],
        }
    }

    fn instance(&self) -> Option<ComputerInstance> {
        self.instance.clone()
    }

    #[cfg(unix)]
    fn open_egress(&mut self) -> Result<crate::egress::EgressEndpoint> {
        self.begin_open_egress()?.wait()
    }

    #[cfg(unix)]
    fn begin_open_egress(&mut self) -> Result<Box<dyn crate::egress::PendingEgressOpen>> {
        let (state, _) = self.require_created()?;
        if state != ComputerState::Running {
            return Err(ComputerError::WrongState {
                expected: ComputerState::Running,
                actual: state,
            });
        }
        if self.egress_open {
            return Err(ComputerError::Backend(
                "the egress stream is already open".to_string(),
            ));
        }
        let (host, guest) = std::os::unix::net::UnixStream::pair()
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        self.egress_open = true;
        self.egress_peer.put(Some(guest));
        Ok(Box::new(SlowOpen {
            delay: self.egress_open_delay,
            endpoint: host,
        }))
    }

    fn close_egress(&mut self) -> Result<()> {
        self.close_egress_now();
        Ok(())
    }

    fn input_available(&self) -> bool {
        self.input_enabled && matches!(self.state, Some(ComputerState::Running))
    }

    fn input_capabilities(&self) -> InputCapabilities {
        let on = self.input_available();
        InputCapabilities {
            pointer: on,
            keyboard: on,
            screenshot: on,
            max_text_len: pegoles_protocol::limits::MAX_TYPE_CHARS,
            kind: InputBackendKind::Test,
        }
    }

    fn input_execute(&mut self, _request_id: &str, op: &InputOp) -> InputOutcome {
        if !self.input_available() {
            return InputOutcome::failed("mock input plane disabled");
        }
        match op {
            InputOp::Down { button, .. } => self.pressed.press_button(*button),
            InputOp::Up { button, .. } => self.pressed.release_button(*button),
            _ => {}
        }
        self.input_log.push(op.clone());
        InputOutcome::ok(0)
    }

    fn input_release_all(&mut self) {
        let at = crate::coords::GuestPoint { x: 0, y: 0 };
        for op in self.pressed.release_all_ops(at) {
            self.input_log.push(op);
        }
    }

    fn input_pressed(&self) -> PressedState {
        self.pressed.clone()
    }

    /// With its input plane on, the mock is a test guest with a 64x36
    /// screen (the size of the frames it captures).
    fn graphical_session(&self) -> crate::guest::GraphicalSessionInfo {
        if !self.input_available() {
            return crate::guest::GraphicalSessionInfo::default();
        }
        crate::guest::GraphicalSessionInfo {
            state: pegoles_protocol::GraphicalSessionState::Ready,
            compositor: Some("mock".to_string()),
            width_px: Some(64),
            height_px: Some(36),
            ..Default::default()
        }
    }

    fn input_capture_frame(
        &mut self,
        _request_id: &str,
        _timeout: std::time::Duration,
    ) -> Result<crate::input::CapturedFrame> {
        use crate::input::{CapturedFrame, FrameCache};
        use pegoles_protocol::{FrameEncoding, FrameId, ObservedFrameMeta};
        if !self.input_available() {
            return Err(ComputerError::UnsupportedOperation(
                "mock input plane disabled".to_string(),
            ));
        }
        let id = self.id.ok_or(ComputerError::NotCreated)?;
        let bytes = crate::input::TestInput::test_pattern_rgba(64, 36);
        let mut cache = FrameCache::new();
        let (frame_id, _) = cache.insert(id, 64, 36, FrameEncoding::RawRgba, bytes.clone(), 0);
        let _ = frame_id;
        Ok(CapturedFrame {
            meta: ObservedFrameMeta {
                frame_id: FrameId::new(),
                computer_id: id,
                captured_at: chrono::Utc::now(),
                width_px: 64,
                height_px: 36,
                encoding: FrameEncoding::RawRgba,
                byte_len: bytes.len() as u64,
                capture_latency_ms: 0,
            },
            bytes,
        })
    }
}

#[cfg(unix)]
struct SlowOpen {
    delay: std::time::Duration,
    endpoint: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl crate::egress::PendingEgressOpen for SlowOpen {
    fn wait(self: Box<Self>) -> Result<crate::egress::EgressEndpoint> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        Ok(crate::egress::EgressEndpoint::Unix(self.endpoint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn created() -> MockComputerBackend {
        let mut b = MockComputerBackend::new();
        b.create(ComputerConfig::default()).unwrap();
        b
    }

    #[test]
    fn create_starts_stopped() {
        let b = created();
        assert_eq!(b.state(), ComputerState::Stopped);
        assert!(b.id().is_some());
    }

    #[test]
    fn double_create_fails() {
        let mut b = created();
        assert!(matches!(
            b.create(ComputerConfig::default()),
            Err(ComputerError::AlreadyCreated(_))
        ));
    }

    #[test]
    fn full_lifecycle() {
        let mut b = created();
        assert_eq!(b.start().unwrap(), ComputerState::Running);
        assert_eq!(b.pause().unwrap(), ComputerState::Paused);
        assert_eq!(b.resume().unwrap(), ComputerState::Running);
        assert_eq!(b.stop().unwrap(), ComputerState::Stopped);
    }

    #[test]
    fn invalid_transitions_error() {
        let mut b = created();
        // pause while stopped
        assert!(matches!(
            b.pause(),
            Err(ComputerError::InvalidTransition { .. })
        ));
        // resume while stopped
        assert!(matches!(
            b.resume(),
            Err(ComputerError::InvalidTransition { .. })
        ));
        b.start().unwrap();
        // start while running
        assert!(matches!(
            b.start(),
            Err(ComputerError::InvalidTransition { .. })
        ));
        b.pause().unwrap();
        // pause while paused
        assert!(matches!(
            b.pause(),
            Err(ComputerError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn ops_before_create_fail() {
        let mut b = MockComputerBackend::new();
        assert_eq!(b.start().unwrap_err(), ComputerError::NotCreated);
        assert_eq!(b.pause().unwrap_err(), ComputerError::NotCreated);
        assert_eq!(b.stop().unwrap_err(), ComputerError::NotCreated);
        assert_eq!(b.resume().unwrap_err(), ComputerError::NotCreated);
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let mut b = created();
        b.start().unwrap();
        let snap = b.snapshot().unwrap();
        b.pause().unwrap();
        assert_eq!(b.restore(snap).unwrap(), ComputerState::Running);
    }

    #[test]
    fn restore_unknown_snapshot_fails() {
        let mut b = created();
        assert!(matches!(
            b.restore(SnapshotId(999)),
            Err(ComputerError::UnknownSnapshot(_))
        ));
    }

    #[test]
    fn reset_returns_to_stopped() {
        let mut b = created();
        b.start().unwrap();
        b.pause().unwrap();
        assert_eq!(b.reset().unwrap(), ComputerState::Stopped);
    }

    #[test]
    fn instance_is_ephemeral_computer_is_persistent() {
        // The core Phase 3.5 invariant: ComputerId survives restarts while
        // every start mints a fresh ephemeral instance handle.
        let mut b = created();
        let computer = b.id().unwrap();
        assert!(b.instance().is_none());
        b.start().unwrap();
        let first = b.instance().expect("instance while running");
        assert_eq!(first.computer_id, computer);
        b.stop().unwrap();
        assert!(b.instance().is_none());
        assert_eq!(b.id(), Some(computer));
        b.start().unwrap();
        let second = b.instance().expect("instance while running");
        assert_eq!(second.computer_id, computer);
        assert_ne!(
            first.instance_id, second.instance_id,
            "restart must mint a new instance, never reuse the handle"
        );
    }

    #[test]
    fn mock_capabilities_describe_reality() {
        let b = created();
        let caps = b.capabilities();
        assert!(caps.pause && caps.resume && caps.snapshot);
        assert!(!caps.vsock && !caps.graphical_display);
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(v["disk_formats"], serde_json::json!(["raw"]));
    }

    #[cfg(unix)]
    #[test]
    fn mock_egress_is_a_socketpair_one_stream_at_a_time() {
        use std::io::{Read, Write};
        let mut b = created();
        assert!(matches!(
            b.open_egress().unwrap_err(),
            ComputerError::WrongState { .. }
        ));
        b.start().unwrap();
        let mut ep = b.open_egress().unwrap();
        let mut peer = b.take_egress_peer().unwrap();
        ep.write_all(b"hi").unwrap();
        let mut buf = [0u8; 2];
        peer.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
        assert!(b.open_egress().is_err(), "one stream per computer");
        b.close_egress().unwrap();
        b.stop().unwrap();
        b.start().unwrap();
        assert!(b.open_egress().is_ok());
    }
}

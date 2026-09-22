//! Registry owning the active computer backend + emitting state events.
//!
//! Phase 2 owns a single computer. Backend selection:
//! macOS Apple Silicon (and no explicit mock override) -> real VM backend;
//! everything else (CI, tests, other platforms) -> Mock.
//! A real-backend failure is surfaced, never silently replaced by Mock.

use chrono::Utc;
use pegoles_computer::{
    default_config, pegoles_data_dir,
    platform::{create_backend, BackendKind},
    ComputerBackend, ComputerError, ComputerImage, ComputerImageManager, GuestObservation,
    ImageStatus, PrepareStage,
};
use pegoles_protocol::{
    AgentEvent, ComputerConfig, ComputerId, ComputerInfo, ComputerState, GuestRuntimeState,
};
use std::path::PathBuf;
use std::time::Duration;

use crate::display::DisplaySlot;
use crate::error::{CoreError, Result};
use crate::events::EventBus;

/// Explicit `PEGOLES_BACKEND=mock` forces Mock (dev/tests). Production
/// selection: macOS arm64 -> MacOSVirtualization, Windows x86_64 ->
/// WindowsHcs (gated by host support at construction), everything else ->
/// Mock. Backend identity itself lives in
/// `pegoles_computer::platform::BackendKind` (re-exported through
/// `pegoles_core`); the Core never names adapters and construction goes
/// through `platform::create_backend` — a gated-out backend returns an
/// explicit error, never a silent Mock.
pub fn default_backend_kind() -> BackendKind {
    use pegoles_computer::platform::{
        host_architecture, host_platform, HostArchitecture, HostPlatform,
    };
    let forced_mock = std::env::var("PEGOLES_BACKEND")
        .map(|v| v == "mock")
        .unwrap_or(false);
    if forced_mock {
        return BackendKind::Mock;
    }
    match (host_platform(), host_architecture()) {
        (HostPlatform::MacOS, HostArchitecture::Arm64) => BackendKind::MacOSVirtualization,
        (HostPlatform::Windows, HostArchitecture::X86_64) => BackendKind::WindowsHcs,
        _ => BackendKind::Mock,
    }
}

pub struct ComputerRegistry {
    pub(crate) backend: Option<Box<dyn ComputerBackend>>,
    /// Platform framebuffer adapter (Phase 4) + display/control state.
    /// `UnavailableDisplay` until an embedding host installs a real one
    /// (macOS desktop). Core owns it so display lifecycle follows computer
    /// lifecycle in ONE place (see `display.rs`).
    pub(crate) display: DisplaySlot,
    kind: BackendKind,
    images: ComputerImageManager,
    data_dir: PathBuf,
    computers_dir: PathBuf,
    pub(crate) bus: EventBus,
    // --- Phase 5 Eyes & Hands state ---
    /// Cooperative cancel for the running agent sequence.
    pub(crate) agent_cancel: crate::input::CancellationToken,
    /// Burst brake shared by all agent actions on this computer.
    pub(crate) rate_limiter: pegoles_computer::ActionRateLimiter,
    /// Bounded structured audit log (content-free rows).
    pub(crate) audit: crate::input::AuditLog,
    /// Deduplicating frame store (pixels out-of-band, never in events).
    pub(crate) frame_cache: pegoles_computer::FrameCache,
    /// Guest buttons/modifiers the host believes are DOWN.
    pub(crate) input_pressed: pegoles_computer::PressedState,
    /// Last pointer position in guest pixels (release targeting).
    pub(crate) last_pointer_guest: pegoles_computer::GuestPoint,
    /// Last published input availability (edge-triggered events).
    pub(crate) last_input_available: Option<bool>,
}

impl std::fmt::Debug for ComputerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputerRegistry")
            .field("kind", &self.kind)
            .field("created", &self.is_created())
            .field("data_dir", &self.data_dir)
            .finish()
    }
}

impl ComputerRegistry {
    /// CI/test/legacy constructor: Mock backend, default data dirs.
    /// No disk IO happens until a real backend is created.
    pub fn new(bus: EventBus) -> Self {
        Self::with_dirs(bus, BackendKind::Mock, pegoles_data_dir())
    }

    /// Production constructor with explicit backend selection.
    pub fn with_backend_kind(bus: EventBus, kind: BackendKind) -> Self {
        Self::with_dirs(bus, kind, pegoles_data_dir())
    }

    /// Test constructor with explicit dirs (never touches real data).
    pub fn with_dirs(bus: EventBus, kind: BackendKind, data_dir: PathBuf) -> Self {
        Self {
            backend: None,
            display: DisplaySlot::unavailable(),
            kind,
            images: ComputerImageManager::new(data_dir.join("images")),
            computers_dir: data_dir.join("computers"),
            data_dir,
            bus,
            agent_cancel: crate::input::CancellationToken::new(),
            rate_limiter: pegoles_computer::ActionRateLimiter::new(
                pegoles_protocol::limits::MAX_ACTIONS_PER_SEC,
            ),
            audit: crate::input::AuditLog::new(),
            frame_cache: pegoles_computer::FrameCache::new(),
            input_pressed: pegoles_computer::PressedState::default(),
            last_pointer_guest: pegoles_computer::GuestPoint { x: 0, y: 0 },
            last_input_available: None,
        }
    }

    pub fn backend_kind(&self) -> BackendKind {
        self.kind
    }

    /// Governor recommendation for THIS host (profile-aware, validated).
    /// Advisory only: `create` still takes explicit configs. The UI and
    /// bench tooling use this to avoid host-starving defaults.
    pub fn suggested_config(
        &self,
        profile: pegoles_computer::PerformanceProfile,
    ) -> pegoles_computer::ComputerRecommendation {
        let host = pegoles_computer::detect_host_resources();
        pegoles_computer::recommend(&host, profile, None)
    }

    /// Guest architecture for display/scheduling. From the live backend's
    /// capabilities when a computer exists, else the platform default.
    /// Never a hardcoded string in the UI layer.
    pub fn guest_arch(&self) -> pegoles_computer::GuestArchitecture {
        use pegoles_computer::{GuestArchitecture, HostArchitecture};
        if let Some(backend) = self.backend.as_ref() {
            return backend.capabilities().guest_arch;
        }
        match pegoles_computer::platform::host_architecture() {
            HostArchitecture::Arm64 => GuestArchitecture::Arm64,
            HostArchitecture::X86_64 => GuestArchitecture::X86_64,
        }
    }

    /// `<data>/images` (same dir the registry's image manager uses).
    pub fn images_dir(&self) -> PathBuf {
        self.data_dir.join("images")
    }

    pub fn image_status(&self) -> ImageStatus {
        self.images.status()
    }

    /// Download + verify + extract the base image, reporting real progress.
    /// Long-running; Tauri runs it on a background thread.
    pub fn prepare_image(
        &self,
        progress: &mut dyn FnMut(PrepareStage, u64, u64),
    ) -> std::result::Result<ComputerImage, pegoles_computer::ComputerError> {
        self.images.prepare(progress)
    }

    pub fn serial_log_path(&self) -> Option<PathBuf> {
        self.backend.as_ref().and_then(|b| b.serial_log_path())
    }

    /// Last lines of the guest serial console (real backend only).
    pub fn read_boot_log(&self, max_lines: usize) -> Result<BootLog> {
        let path = self.serial_log_path().ok_or(CoreError::NoBootLog)?;
        let content = std::fs::read_to_string(&path).map_err(|e| {
            CoreError::Computer(ComputerError::Backend(format!("cannot read boot log: {e}")))
        })?;
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let total = lines.len();
        let start = total.saturating_sub(max_lines.max(1));
        Ok(BootLog {
            available: true,
            total_lines: total,
            tail: lines[start..].to_vec(),
        })
    }

    pub fn is_created(&self) -> bool {
        self.backend.as_ref().and_then(|b| b.id()).is_some()
    }

    pub fn info(&self) -> Option<ComputerInfo> {
        self.backend.as_ref().and_then(|b| {
            b.id().map(|id| ComputerInfo {
                id,
                state: b.state(),
                config: b.config().unwrap_or_else(default_config),
            })
        })
    }

    pub fn state(&self) -> Option<ComputerState> {
        self.backend.as_ref().map(|b| b.state())
    }

    pub fn computer_id(&self) -> Result<ComputerId> {
        self.backend
            .as_ref()
            .and_then(|b| b.id())
            .ok_or(CoreError::NoComputer)
    }

    fn make_backend(&self) -> Result<Box<dyn ComputerBackend>> {
        // Central factory: Real-family kinds on unsupported platforms
        // return UnsupportedPlatform here, never a silent Mock.
        create_backend(
            self.kind,
            self.data_dir.join("images"),
            self.computers_dir.clone(),
        )
        .map_err(CoreError::Computer)
    }

    /// Create the computer with an explicit config (validated by the
    /// backend; a framebuffer is refused when the backend has none).
    pub fn create(&mut self, config: ComputerConfig) -> Result<ComputerId> {
        self.ensure_no_computer()?;
        let backend = self.make_backend()?;
        self.create_on(backend, config)
    }

    /// Create with the product default: `default_config()` plus the
    /// default framebuffer (`display::default_display_config`: DesktopLarge
    /// when the ready image is graphical, the backend can attach graphics
    /// and a display adapter is installed; headless otherwise).
    pub fn create_default(&mut self) -> Result<ComputerId> {
        self.ensure_no_computer()?;
        let backend = self.make_backend()?;
        self.create_default_on(backend)
    }

    /// `create_default` on an injected backend (tests / embedding).
    pub fn create_default_on(&mut self, backend: Box<dyn ComputerBackend>) -> Result<ComputerId> {
        let mut config = default_config();
        config.display = crate::display::default_display_config(
            self.image_is_graphical(),
            backend.capabilities().graphical_display,
            self.display.backend.is_available(),
        );
        self.create_on(backend, config)
    }

    /// Whether the READY derived image declares a graphical stack
    /// (manifest `graphical`, recorded by the image builder from what it
    /// actually installed). Missing/invalid image = false (headless).
    pub fn image_is_graphical(&self) -> bool {
        self.images
            .derived_manifest()
            .map(|m| m.graphical.is_some())
            .unwrap_or(false)
    }

    fn ensure_no_computer(&self) -> Result<()> {
        match self.backend.as_ref() {
            Some(b) => Err(CoreError::Computer(match b.id() {
                Some(id) => ComputerError::AlreadyCreated(id),
                None => ComputerError::Backend("computer slot busy".to_string()),
            })),
            None => Ok(()),
        }
    }

    /// Create on an injected backend (tests / embedding hosts). Same
    /// checks and events as `create`.
    pub fn create_on(
        &mut self,
        mut backend: Box<dyn ComputerBackend>,
        config: ComputerConfig,
    ) -> Result<ComputerId> {
        self.ensure_no_computer()?;
        if config.display.is_some() && !backend.capabilities().graphical_display {
            return Err(CoreError::Computer(ComputerError::InvalidConfig(
                "this backend cannot attach a display device".to_string(),
            )));
        }
        if let Some(display) = config.display {
            // Backends validate the full config; the framebuffer is also
            // checked here so every backend kind shares the same rule.
            display.validate().map_err(|e| {
                CoreError::Computer(ComputerError::InvalidConfig(format!(
                    "display rejected: {e:?}"
                )))
            })?;
        }
        let id = backend.create(config)?;
        self.backend = Some(backend);
        self.bus.publish(AgentEvent::ComputerCreated {
            computer_id: id,
            at: Utc::now(),
        });
        self.bus.publish(AgentEvent::ComputerStateChanged {
            computer_id: id,
            from: ComputerState::Stopped,
            to: ComputerState::Stopped,
            at: Utc::now(),
        });
        Ok(id)
    }

    fn mutate(
        &mut self,
        op: impl FnOnce(&mut dyn ComputerBackend) -> std::result::Result<ComputerState, ComputerError>,
    ) -> Result<ComputerState> {
        let backend = self.backend.as_deref_mut().ok_or(CoreError::NoComputer)?;
        let id = backend.id().ok_or(CoreError::NoComputer)?;
        let from = backend.state();
        let to = op(backend)?;
        self.bus.publish(AgentEvent::ComputerStateChanged {
            computer_id: id,
            from,
            to,
            at: Utc::now(),
        });
        Ok(to)
    }

    pub fn start(&mut self) -> Result<ComputerState> {
        let state = self.mutate(|b| b.start())?;
        self.display_on_started();
        // A fresh VM start opens the guest-readiness window. Backends
        // without guest support stay Unavailable (no event for Mock).
        if self.guest_state() == GuestRuntimeState::Waiting {
            if let Ok(id) = self.computer_id() {
                self.bus.publish(AgentEvent::GuestRuntimeWaiting {
                    computer_id: id,
                    at: Utc::now(),
                });
            }
        }
        // Auto-attach when the UI already reported a framebuffer slot.
        let mut events = Vec::new();
        self.reconcile_display(Some(state), &mut events);
        Ok(state)
    }

    /// Stop: input returns to Pegoles and the framebuffer view is removed
    /// BEFORE the VM stops. If the stop fails while the VM keeps running,
    /// the next pump re-attaches (the UI slot is remembered).
    pub fn stop(&mut self) -> Result<ComputerState> {
        self.computer_id()?;
        self.cancel_agent_input("computer stopping");
        let guest_before = self.guest_state();
        let mut events = Vec::new();
        self.display_on_leaving("computer_stopped", &mut events);
        let state = self.mutate(|b| b.stop())?;
        self.display_on_stopped();
        self.emit_guest_gone_if_needed(guest_before, "vm_stopped");
        Ok(state)
    }

    /// Pause: control auto-returns; the view stays (dimmed, last frame).
    /// A running agent sequence is interrupted (never auto-resumed:
    /// partial actions are not replayed).
    pub fn pause(&mut self) -> Result<ComputerState> {
        self.cancel_agent_input("computer pausing");
        let state = self.mutate(|b| b.pause())?;
        let mut events = Vec::new();
        self.reconcile_display(Some(state), &mut events);
        Ok(state)
    }

    /// Resume: the view returns to normal presentation.
    pub fn resume(&mut self) -> Result<ComputerState> {
        let state = self.mutate(|b| b.resume())?;
        let mut events = Vec::new();
        self.reconcile_display(Some(state), &mut events);
        Ok(state)
    }

    pub fn reset(&mut self) -> Result<ComputerState> {
        self.computer_id()?;
        self.cancel_agent_input("computer resetting");
        let guest_before = self.guest_state();
        let mut events = Vec::new();
        self.display_on_leaving("computer_reset", &mut events);
        let state = self.mutate(|b| b.reset())?;
        self.display_on_stopped();
        self.emit_guest_gone_if_needed(guest_before, "vm_reset");
        Ok(state)
    }

    /// Destroy the computer (stop + delete its private resources). The
    /// display is detached and the remembered UI slot forgotten; a new
    /// computer can be created afterwards.
    pub fn destroy(&mut self) -> Result<()> {
        let id = self.computer_id()?;
        self.cancel_agent_input("computer destroyed");
        let from = self.state().unwrap_or(ComputerState::Stopped);
        let guest_before = self.guest_state();
        let mut events = Vec::new();
        self.display_on_leaving("computer_destroyed", &mut events);
        self.display.desired = None;
        let backend = self.backend.as_deref_mut().ok_or(CoreError::NoComputer)?;
        backend.destroy()?;
        self.emit_guest_gone_if_needed(guest_before, "vm_destroyed");
        self.backend = None;
        self.display_on_stopped();
        self.bus.publish(AgentEvent::ComputerStateChanged {
            computer_id: id,
            from,
            to: ComputerState::Stopped,
            at: Utc::now(),
        });
        Ok(())
    }

    fn emit_guest_gone_if_needed(&mut self, before: GuestRuntimeState, reason: &str) {
        use GuestRuntimeState as G;
        if matches!(
            before,
            G::Waiting | G::Connecting | G::Ready | G::Disconnected | G::Incompatible | G::Error
        ) {
            if let Ok(id) = self.computer_id() {
                self.bus.publish(AgentEvent::GuestRuntimeDisconnected {
                    computer_id: id,
                    reason: reason.to_string(),
                    at: Utc::now(),
                });
            }
        }
    }

    // --- guest control plane ---

    pub fn guest_state(&self) -> GuestRuntimeState {
        self.backend
            .as_ref()
            .map(|b| b.guest_state())
            .unwrap_or(GuestRuntimeState::Unavailable)
    }

    pub fn guest_ready_ms(&self) -> Option<u64> {
        self.backend.as_ref().and_then(|b| b.guest_ready_ms())
    }

    pub fn guest_info(&self) -> Option<pegoles_guest_proto::SystemInfo> {
        self.backend.as_ref().and_then(|b| b.guest_info())
    }

    /// Blocking Ping -> Pong round-trip (latency in ms).
    pub fn guest_ping(&mut self, timeout: Duration) -> Result<u64> {
        let backend = self.backend.as_deref_mut().ok_or(CoreError::NoComputer)?;
        backend.guest_ping(timeout).map_err(CoreError::Computer)
    }

    /// Blocking GetSystemInfo round-trip.
    pub fn guest_info_request(
        &mut self,
        timeout: Duration,
    ) -> Result<pegoles_guest_proto::SystemInfo> {
        let backend = self.backend.as_deref_mut().ok_or(CoreError::NoComputer)?;
        backend
            .guest_info_request(timeout)
            .map_err(CoreError::Computer)
    }

    /// Drain guest transport + heartbeat maintenance, publishing one
    /// AgentEvent per observation. Call on every status poll; never blocks.
    /// Heartbeat Ping/Pong stays silent (no UI events by design).
    /// Returns the published events so Tauri can mirror them into history.
    pub fn pump_guest(&mut self) -> Vec<AgentEvent> {
        let backend = match self.backend.as_deref_mut() {
            Some(b) => b,
            None => return Vec::new(),
        };
        let id = match backend.id() {
            Some(id) => id,
            None => return Vec::new(),
        };
        let configured = backend
            .config()
            .and_then(|c| c.display)
            .map(|d| (d.width_px, d.height_px));
        let mut published = Vec::new();
        for obs in backend.poll_guest() {
            let event: Option<AgentEvent> = match obs {
                GuestObservation::Connected => Some(AgentEvent::GuestRuntimeConnected {
                    computer_id: id,
                    at: Utc::now(),
                }),
                GuestObservation::HandshakeCompleted { protocol_version } => {
                    Some(AgentEvent::GuestHandshakeCompleted {
                        computer_id: id,
                        protocol_version,
                        at: Utc::now(),
                    })
                }
                GuestObservation::BecameReady { ready_in_ms } => {
                    Some(AgentEvent::GuestRuntimeReady {
                        computer_id: id,
                        ready_in_ms,
                        at: Utc::now(),
                    })
                }
                GuestObservation::StateChanged { to, detail, .. } => {
                    use GuestRuntimeState as G;
                    match to {
                        G::Disconnected => Some(AgentEvent::GuestRuntimeDisconnected {
                            computer_id: id,
                            reason: detail.unwrap_or_else(|| "disconnected".to_string()),
                            at: Utc::now(),
                        }),
                        G::Incompatible => {
                            // Guest version rides in the detail string
                            // ("guest protocol N"); extract best effort.
                            let guest_version = detail
                                .as_deref()
                                .and_then(|d| d.split_whitespace().last()?.parse::<u32>().ok())
                                .unwrap_or(0);
                            Some(AgentEvent::GuestRuntimeIncompatible {
                                computer_id: id,
                                guest_version,
                                at: Utc::now(),
                            })
                        }
                        G::Error => Some(AgentEvent::GuestRuntimeError {
                            computer_id: id,
                            message: detail.unwrap_or_else(|| "guest error".to_string()),
                            at: Utc::now(),
                        }),
                        _ => None,
                    }
                }
                GuestObservation::GraphicalSessionChanged(change) => {
                    graphical_event(id, change, configured)
                }
                GuestObservation::InfoReceived(_) | GuestObservation::PongReceived { .. } => None,
                // Input acks and frame chunks belong to a blocking
                // executor wait (consumed via wait + deferral). Orphaned
                // here (no waiter): drop silently, never log spam.
                GuestObservation::InputAckReceived { .. }
                | GuestObservation::FrameBeginReceived { .. }
                | GuestObservation::FrameChunkReceived { .. } => None,
            };
            if let Some(event) = event {
                self.bus.publish(event.clone());
                published.push(event);
            }
        }
        // Edge-triggered input availability (UI gating, no spam).
        let mut cap_out = Vec::new();
        self.publish_input_capability(&mut cap_out);
        for event in cap_out {
            self.bus.publish(event.clone());
            published.push(event);
        }
        published
    }
}

/// Map a guest graphical-session transition to its AgentEvent. Ready and
/// Failed are events; Starting/Unavailable only move the derived
/// viewport. Size: as reported by the guest, else the configured scanout
/// (real VM config), else 0 — never invented.
fn graphical_event(
    computer_id: ComputerId,
    change: pegoles_computer::GraphicalSessionChange,
    configured: Option<(u32, u32)>,
) -> Option<AgentEvent> {
    use pegoles_protocol::GraphicalSessionState as S;
    match change.to {
        S::Ready => Some(AgentEvent::GraphicalSessionReady {
            computer_id,
            compositor: change.compositor.unwrap_or_else(|| "unknown".to_string()),
            width_px: change.width_px.or(configured.map(|c| c.0)).unwrap_or(0),
            height_px: change.height_px.or(configured.map(|c| c.1)).unwrap_or(0),
            ready_in_ms: change.at_ms,
            at: Utc::now(),
        }),
        S::Failed => Some(AgentEvent::GraphicalSessionFailed {
            computer_id,
            message: change
                .detail
                .unwrap_or_else(|| "graphical session failed".to_string()),
            at: Utc::now(),
        }),
        S::Starting | S::Unavailable => None,
    }
}

/// Snapshot of the guest serial console for the UI.
#[derive(Clone, Debug)]
pub struct BootLog {
    pub available: bool,
    pub total_lines: usize,
    pub tail: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_lifecycle_emits_state() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let mut r = ComputerRegistry::new(bus);
        assert!(!r.is_created());
        r.create(ComputerConfig::default()).unwrap();
        assert!(r.is_created());
        r.start().unwrap();
        assert_eq!(r.state(), Some(ComputerState::Running));
        // at least created + state events arrived
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn ops_without_computer_fail() {
        let mut r = ComputerRegistry::new(EventBus::new());
        assert!(matches!(r.start(), Err(CoreError::NoComputer)));
    }

    #[test]
    fn mock_is_default_kind_for_tests() {
        let r = ComputerRegistry::new(EventBus::new());
        assert_eq!(r.backend_kind(), BackendKind::Mock);
    }

    #[test]
    fn boot_log_unavailable_without_real_backend() {
        let mut r = ComputerRegistry::new(EventBus::new());
        r.create(ComputerConfig::default()).unwrap();
        assert!(matches!(r.read_boot_log(10), Err(CoreError::NoBootLog)));
    }

    #[test]
    fn mock_backend_has_no_guest_plane() {
        let mut r = ComputerRegistry::new(EventBus::new());
        r.create(ComputerConfig::default()).unwrap();
        r.start().unwrap();
        // Mock never leaves Unavailable: no Waiting event, no observations.
        assert_eq!(
            r.guest_state(),
            pegoles_protocol::GuestRuntimeState::Unavailable
        );
        // Phase 5: the first pump emits exactly one edge-triggered
        // InputCapabilityChanged(unavailable); afterwards it stays silent.
        let first = r.pump_guest();
        assert_eq!(first.len(), 1);
        assert!(matches!(
            first[0],
            AgentEvent::InputCapabilityChanged {
                available: false,
                ..
            }
        ));
        assert!(r.pump_guest().is_empty());
        assert_eq!(r.guest_ready_ms(), None);
        assert!(r.guest_info().is_none());
    }

    #[test]
    fn start_on_mock_emits_no_guest_waiting() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let mut r = ComputerRegistry::new(bus);
        r.create(ComputerConfig::default()).unwrap();
        // Drain creation events.
        while rx.try_recv().is_ok() {}
        r.start().unwrap();
        // Only the VM state change, never a guest event, on Mock.
        let mut saw_guest = false;
        while let Ok(ev) = rx.try_recv() {
            let s = serde_json::to_string(&ev).unwrap();
            if s.contains("guest") {
                saw_guest = true;
            }
        }
        assert!(!saw_guest);
    }
}

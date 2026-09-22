//! Display lifecycle + control ownership, end to end through Core with
//! the real native engine (scripted `FakeTransport`) and `TestDisplay`.
//! No hypervisor, no window — every fact is scripted, nothing is timed.

use pegoles_computer::native_backend::SharedFakeTransport;
use pegoles_computer::{
    default_config, platform::BackendKind, ComputerError, DisplayEvent, DisplayGeometry,
    DisplayPresentation, DisplayRect, MacOSVirtualizationBackend, TestDisplay, TestDisplayCall,
};
use pegoles_protocol::{
    AgentEvent, ComputerConfig, ComputerId, ComputerState, ControlOwner, DisplayConfig,
    DisplayProfile, GuestRuntimeState, ViewportState,
};
use std::path::Path;
use tokio::sync::broadcast;

use crate::display::{default_display_config, DisplayBounds, GeometryOutcome};
use crate::error::CoreError;
use crate::events::EventBus;
use crate::registry::ComputerRegistry;
use crate::viewport::ViewportIssue;

const BOUNDS: DisplayBounds = DisplayBounds {
    width: 1600.0,
    height: 1000.0,
    scale: 2.0,
};

const GFX_STARTING: &str =
    r#"{"type":"graphical_session","status":"starting","compositor":"weston"}"#;
const GFX_READY: &str = r#"{"type":"graphical_session","status":"ready","compositor":"weston","width_px":1440,"height_px":900}"#;
const GFX_FAILED: &str = r#"{"type":"graphical_session","status":"failed","compositor":"weston","detail":"compositor exited"}"#;
const HELLO: &str = r#"{"type":"guest_hello","protocol_version":1,"runtime_version":"0.2.0","os":"debian","os_version":"13","arch":"aarch64"}"#;

fn geo(x: f64, y: f64, w: f64, h: f64) -> DisplayGeometry {
    DisplayGeometry {
        rect: DisplayRect {
            x,
            y,
            width: w,
            height: h,
        },
        visible: true,
        animate_ms: 0,
    }
}

fn slot() -> DisplayGeometry {
    geo(240.0, 80.0, 1152.0, 720.0)
}

fn seed_official_image(images: &Path) {
    let raw = pegoles_computer::ComputerImageManager::new(images.to_path_buf()).base_raw_path();
    std::fs::create_dir_all(raw.parent().expect("image dir")).unwrap();
    std::fs::write(&raw, b"fake-disk").unwrap();
    std::fs::write(raw.with_file_name("base.raw.verified"), "abc").unwrap();
}

/// Registry + scripted VM host + recording display, computer created.
struct Rig {
    _tmp: tempfile::TempDir,
    registry: ComputerRegistry,
    fake: SharedFakeTransport,
    display: TestDisplay,
    rx: broadcast::Receiver<AgentEvent>,
    id: ComputerId,
}

impl Rig {
    fn graphical() -> Self {
        Self::with_config(
            DisplayConfig::for_profile(DisplayProfile::DesktopLarge),
            true,
        )
    }

    fn headless() -> Self {
        Self::with_config(None, true)
    }

    fn with_config(display_config: Option<DisplayConfig>, install_display: bool) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().to_path_buf();
        seed_official_image(&data.join("images"));
        let bus = EventBus::new();
        let rx = bus.subscribe();
        let mut registry = ComputerRegistry::with_dirs(bus, BackendKind::Mock, data.clone());
        let display = TestDisplay::new();
        if install_display {
            registry.install_display_backend(Box::new(display.clone()));
        }
        let fake = SharedFakeTransport::new();
        fake.lock().graphical_session_reply = None;
        let backend = MacOSVirtualizationBackend::with_transport(
            data.join("images"),
            data.join("computers"),
            Box::new(fake.clone()),
        );
        let mut config: ComputerConfig = default_config();
        config.display = display_config;
        let id = registry.create_on(Box::new(backend), config).unwrap();
        let mut rig = Self {
            _tmp: tmp,
            registry,
            fake,
            display,
            rx,
            id,
        };
        rig.events();
        rig
    }

    fn cid(&self) -> String {
        self.id.to_string()
    }

    /// Everything published on the bus since the last call.
    fn events(&mut self) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn viewport(&self) -> ViewportState {
        self.registry.viewport(false).state
    }

    fn guest_ready(&mut self) {
        {
            let cid = self.cid();
            let mut f = self.fake.lock();
            f.inject_guest_connected(&cid);
            f.inject_guest_frame(&cid, HELLO);
            f.inject_guest_frame(&cid, r#"{"type":"ready"}"#);
        }
        self.registry.pump();
        assert_eq!(self.registry.guest_state(), GuestRuntimeState::Ready);
    }

    fn report(&mut self, frame: &str) {
        let cid = self.cid();
        self.fake.lock().inject_guest_frame(&cid, frame);
        self.registry.pump();
    }

    /// Start, attach at `slot()`, guest handshake, compositor Ready.
    fn boot_display_ready(&mut self) {
        self.registry.start().unwrap();
        assert_eq!(
            self.registry.set_display_geometry(slot(), BOUNDS).unwrap(),
            GeometryOutcome::Attached
        );
        self.guest_ready();
        self.report(GFX_READY);
        assert_eq!(self.viewport(), ViewportState::Ready);
    }

    fn attach_calls(&self) -> usize {
        self.display
            .calls()
            .iter()
            .filter(|c| matches!(c, TestDisplayCall::Attach { .. }))
            .count()
    }
}

fn count(events: &[AgentEvent], pred: impl Fn(&AgentEvent) -> bool) -> usize {
    events.iter().filter(|e| pred(e)).count()
}

fn is_display_ready(e: &AgentEvent) -> bool {
    matches!(e, AgentEvent::DisplayReady { .. })
}

fn is_detached(e: &AgentEvent) -> bool {
    matches!(e, AgentEvent::DisplayDetached { .. })
}

fn control_change(e: &AgentEvent) -> Option<(ControlOwner, ControlOwner)> {
    match e {
        AgentEvent::ControlOwnershipChanged { from, to, .. } => Some((*from, *to)),
        _ => None,
    }
}

fn control_changes(events: &[AgentEvent]) -> Vec<(ControlOwner, ControlOwner)> {
    events.iter().filter_map(control_change).collect()
}

// --- readiness chain ---

#[test]
fn readiness_chain_emits_real_events_and_display_ready_once() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    assert_eq!(rig.viewport(), ViewportState::GuestConnecting);
    assert_eq!(
        rig.registry.set_display_geometry(slot(), BOUNDS).unwrap(),
        GeometryOutcome::Attached
    );
    assert!(rig.registry.display_attached());
    let ev = rig.events();
    assert_eq!(
        count(&ev, |e| matches!(e, AgentEvent::DisplayAttached { .. })),
        1
    );
    assert_eq!(count(&ev, is_display_ready), 0, "guest not ready yet");

    rig.guest_ready();
    assert_eq!(rig.viewport(), ViewportState::DisplayStarting);
    // The host asked for the graphical session exactly once.
    let sent = rig.fake.lock().sent.join("\n");
    assert_eq!(sent.matches("get_graphical_session").count(), 1);

    rig.report(GFX_STARTING);
    assert_eq!(rig.viewport(), ViewportState::DisplayStarting);
    let ev = rig.events();
    assert_eq!(count(&ev, is_display_ready), 0);

    rig.report(GFX_READY);
    assert_eq!(rig.viewport(), ViewportState::Ready);
    let ev = rig.events();
    let ready = ev
        .iter()
        .find_map(|e| match e {
            AgentEvent::GraphicalSessionReady {
                compositor,
                width_px,
                height_px,
                ..
            } => Some((compositor.clone(), *width_px, *height_px)),
            _ => None,
        })
        .expect("GraphicalSessionReady");
    assert_eq!(ready, ("weston".to_string(), 1440, 900));
    assert_eq!(count(&ev, is_display_ready), 1);
    let ready_ms = rig.registry.display_ready_ms().expect("ready ms");
    let view = rig.registry.computer_view(false);
    assert_eq!(view.display_ready_ms, Some(ready_ms));
    assert!(view.display_available && view.display_attached);

    // Never twice per boot, however often we poll or re-report.
    rig.registry.pump();
    rig.report(GFX_READY);
    assert_eq!(count(&rig.events(), is_display_ready), 0);
}

#[test]
fn display_ready_requires_attachment_too() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    rig.guest_ready();
    rig.report(GFX_READY);
    // Session ready but no framebuffer slot reported yet.
    assert_eq!(rig.viewport(), ViewportState::DisplayStarting);
    assert_eq!(count(&rig.events(), is_display_ready), 0);
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap();
    assert_eq!(rig.viewport(), ViewportState::Ready);
    assert_eq!(count(&rig.events(), is_display_ready), 1);
}

#[test]
fn graphical_failure_is_an_event_and_an_error_viewport() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.events();
    rig.report(GFX_FAILED);
    let ev = rig.events();
    assert!(ev.iter().any(|e| matches!(
        e,
        AgentEvent::GraphicalSessionFailed { message, .. } if message == "compositor exited"
    )));
    let view = rig.registry.viewport(false);
    assert_eq!(view.state, ViewportState::Error);
    assert_eq!(view.issue, Some(ViewportIssue::GraphicalSessionFailed));
}

// --- geometry ---

#[test]
fn geometry_before_start_is_deferred_then_attached_on_start() {
    let mut rig = Rig::graphical();
    assert_eq!(
        rig.registry.set_display_geometry(slot(), BOUNDS).unwrap(),
        GeometryOutcome::Deferred
    );
    assert_eq!(rig.attach_calls(), 0);
    rig.registry.start().unwrap();
    assert_eq!(rig.attach_calls(), 1);
    assert!(rig.registry.display_attached());
}

#[test]
fn invalid_geometry_is_rejected_before_any_native_call() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    for bad in [
        geo(f64::NAN, 0.0, 10.0, 10.0),
        geo(0.0, 0.0, -5.0, 10.0),
        geo(5000.0, 5000.0, 10.0, 10.0), // fully outside the webview
    ] {
        assert!(matches!(
            rig.registry.set_display_geometry(bad, BOUNDS),
            Err(CoreError::InvalidGeometry(_))
        ));
    }
    let mut slow = slot();
    slow.animate_ms = pegoles_computer::MAX_GEOMETRY_ANIMATION_MS + 1;
    assert!(rig.registry.set_display_geometry(slow, BOUNDS).is_err());
    assert!(rig.display.calls().is_empty());
}

#[test]
fn geometry_is_clamped_and_snapped_before_reaching_native() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    rig.registry
        .set_display_geometry(geo(10.26, -4.0, 2000.0, 300.4), BOUNDS)
        .unwrap();
    match rig.display.calls().first() {
        Some(TestDisplayCall::Attach { geometry, .. }) => {
            assert_eq!(geometry.rect.x, 10.5);
            assert_eq!(geometry.rect.y, 0.0);
            assert_eq!(geometry.rect.width, 1600.0 - 10.5);
        }
        other => panic!("expected attach, got {other:?}"),
    }
}

#[test]
fn viewport_resizes_are_deduplicated() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap();
    rig.display.clear_calls();
    let mut jitter = slot();
    jitter.rect.width += 0.2;
    assert_eq!(
        rig.registry.set_display_geometry(jitter, BOUNDS).unwrap(),
        GeometryOutcome::Unchanged
    );
    assert!(rig.display.calls().is_empty(), "jitter reached native");
    let mut resized = slot();
    resized.rect.width = 900.0;
    resized.rect.height = 562.5;
    assert_eq!(
        rig.registry.set_display_geometry(resized, BOUNDS).unwrap(),
        GeometryOutcome::Updated
    );
    let mut hidden = resized;
    hidden.visible = false;
    assert_eq!(
        rig.registry.set_display_geometry(hidden, BOUNDS).unwrap(),
        GeometryOutcome::Updated,
        "visibility is always a change"
    );
    assert_eq!(rig.display.calls().len(), 2);
}

#[test]
fn headless_computer_is_ready_without_a_display() {
    let mut rig = Rig::headless();
    rig.registry.start().unwrap();
    assert_eq!(
        rig.registry.set_display_geometry(slot(), BOUNDS).unwrap(),
        GeometryOutcome::Deferred
    );
    rig.guest_ready();
    let view = rig.registry.computer_view(false);
    assert_eq!(view.viewport_state, ViewportState::Ready);
    assert_eq!(view.viewport_issue, None);
    assert!(!view.display_available);
    assert_eq!(view.display_config, None);
    assert!(rig.display.calls().is_empty(), "no view for a headless VM");
    assert!(matches!(
        rig.registry.take_control(),
        Err(CoreError::ControlUnavailable(_))
    ));
    assert_eq!(count(&rig.events(), is_display_ready), 0);
}

#[test]
fn graphical_computer_without_adapter_is_degraded_ready() {
    let mut rig = Rig::with_config(
        DisplayConfig::for_profile(DisplayProfile::DesktopLarge),
        false,
    );
    rig.registry.start().unwrap();
    rig.guest_ready();
    rig.report(GFX_READY);
    let view = rig.registry.viewport(false);
    assert_eq!(view.state, ViewportState::Ready);
    assert_eq!(view.issue, Some(ViewportIssue::DisplayBackendUnavailable));
    assert!(rig.registry.take_control().is_err());
}

#[test]
fn attach_failure_is_sticky_until_a_new_geometry() {
    let mut rig = Rig::graphical();
    rig.registry.start().unwrap();
    rig.guest_ready();
    rig.report(GFX_READY);
    rig.display.fail_next_attach("no window");
    assert!(rig.registry.set_display_geometry(slot(), BOUNDS).is_err());
    let view = rig.registry.computer_view(false);
    assert_eq!(view.viewport_state, ViewportState::Error);
    assert_eq!(view.viewport_issue, Some(ViewportIssue::DisplayFailed));
    assert!(view
        .display_error
        .as_deref()
        .unwrap_or("")
        .contains("no window"));
    // Polling never turns into a retry loop.
    rig.registry.pump();
    rig.registry.pump();
    assert_eq!(rig.attach_calls(), 1);
    // The UI retrying (new geometry) is the recovery path.
    assert_eq!(
        rig.registry.set_display_geometry(slot(), BOUNDS).unwrap(),
        GeometryOutcome::Attached
    );
    assert_eq!(rig.viewport(), ViewportState::Ready);
}

#[test]
fn native_failure_event_detaches_and_blocks_auto_attach() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    rig.display.push_event(DisplayEvent::Failed {
        computer_id: rig.id,
        message: "view lost".into(),
    });
    rig.registry.pump();
    let ev = rig.events();
    assert_eq!(count(&ev, is_detached), 1);
    assert_eq!(
        control_changes(&ev),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
    assert_eq!(
        rig.registry.viewport(false).issue,
        Some(ViewportIssue::DisplayFailed)
    );
    rig.registry.pump();
    assert_eq!(rig.attach_calls(), 1);
}

#[test]
fn late_native_facts_are_matched_in_fifo_order() {
    // An asynchronous adapter answers after the fact: attach, stop
    // (detach), start (attach again), THEN the three answers arrive. Only
    // the last attach may confirm; the detach answer is an echo.
    let mut rig = Rig::graphical();
    rig.display.set_auto_events(false);
    rig.registry.start().unwrap();
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap();
    assert!(!rig.registry.display_attached(), "not confirmed yet");
    rig.registry.stop().unwrap();
    rig.registry.start().unwrap();
    assert_eq!(rig.attach_calls(), 2);
    rig.events();
    for fact in [
        DisplayEvent::Attached {
            computer_id: rig.id,
        },
        DisplayEvent::Detached {
            computer_id: rig.id,
            reason: "computer_stopped".into(),
        },
        DisplayEvent::Attached {
            computer_id: rig.id,
        },
    ] {
        rig.display.push_event(fact);
    }
    rig.registry.pump();
    let ev = rig.events();
    assert_eq!(
        count(&ev, |e| matches!(e, AgentEvent::DisplayAttached { .. })),
        1
    );
    assert_eq!(
        count(&ev, is_detached),
        0,
        "never confirmed, never reported"
    );
    assert!(rig.registry.display_attached());
}

#[test]
fn failed_async_attach_owes_no_detach_echo() {
    // attach -> (native fails later) -> stop issues a detach that has
    // nothing to remove -> the Failed answer arrives. A LATER genuine
    // native detach must still be honored (no swallowed fact).
    let mut rig = Rig::graphical();
    rig.display.set_auto_events(false);
    rig.registry.start().unwrap();
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap();
    rig.registry.stop().unwrap();
    rig.display.push_event(DisplayEvent::Failed {
        computer_id: rig.id,
        message: "no window".into(),
    });
    rig.registry.pump();
    rig.registry.start().unwrap(); // new boot: failure forgotten, slot remembered
    rig.registry
        .set_display_geometry(slot(), BOUNDS)
        .expect("retry attaches");
    rig.display.push_event(DisplayEvent::Attached {
        computer_id: rig.id,
    });
    rig.registry.pump();
    assert!(rig.registry.display_attached());
    rig.events();
    rig.display.push_event(DisplayEvent::Detached {
        computer_id: rig.id,
        reason: "window_closed".into(),
    });
    rig.registry.pump();
    assert_eq!(count(&rig.events(), is_detached), 1);
    assert!(!rig.registry.display_attached());
}

// --- lifecycle coupling ---

#[test]
fn pause_dims_and_resume_restores() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.pause().unwrap();
    assert_eq!(rig.viewport(), ViewportState::Paused);
    assert_eq!(
        rig.display.state().presentation,
        DisplayPresentation::Dimmed
    );
    assert!(
        rig.registry.display_attached(),
        "pause keeps the last frame"
    );
    rig.registry.resume().unwrap();
    assert_eq!(
        rig.display.state().presentation,
        DisplayPresentation::Normal
    );
    assert_eq!(rig.viewport(), ViewportState::Ready);
}

#[test]
fn stop_detaches_exactly_once_and_leaves_no_view() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.events();
    rig.registry.stop().unwrap();
    assert_eq!(rig.registry.live_native_views(), 0);
    assert!(!rig.registry.display_attached());
    assert_eq!(rig.viewport(), ViewportState::Off);
    // The native Detached fact that follows is deduplicated.
    rig.registry.pump();
    let ev = rig.events();
    assert_eq!(count(&ev, is_detached), 1);
    assert!(ev.iter().any(|e| matches!(
        e,
        AgentEvent::DisplayDetached { reason, .. } if reason == "computer_stopped"
    )));
    assert_eq!(rig.registry.display_ready_ms(), None);
}

#[test]
fn restart_reattaches_remembered_slot_and_readiness_is_per_boot() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.stop().unwrap();
    rig.events();
    rig.registry.start().unwrap();
    assert!(
        rig.registry.display_attached(),
        "remembered slot re-attached"
    );
    assert_eq!(rig.viewport(), ViewportState::GuestConnecting);
    rig.guest_ready();
    rig.report(GFX_READY);
    assert_eq!(count(&rig.events(), is_display_ready), 1, "new boot");
}

#[test]
fn reset_and_destroy_detach() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.reset().unwrap();
    assert_eq!(rig.registry.live_native_views(), 0);
    rig.registry.start().unwrap();
    assert!(rig.registry.display_attached());
    rig.registry.destroy().unwrap();
    assert_eq!(rig.registry.live_native_views(), 0);
    assert!(!rig.registry.is_created());
    assert_eq!(rig.viewport(), ViewportState::Off);
    // The UI slot is forgotten with the computer.
    let backend = MacOSVirtualizationBackend::with_transport(
        rig._tmp.path().join("images"),
        rig._tmp.path().join("computers"),
        Box::new(SharedFakeTransport::new()),
    );
    let mut cfg = default_config();
    cfg.display = DisplayConfig::for_profile(DisplayProfile::DesktopLarge);
    rig.registry.create_on(Box::new(backend), cfg).unwrap();
    rig.display.clear_calls();
    rig.registry.start().unwrap();
    assert_eq!(rig.attach_calls(), 0);
}

#[test]
fn vm_error_detaches_and_returns_control() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    let cid = rig.cid();
    rig.fake.lock().inject_vm_state(&cid, "error");
    rig.registry.pump();
    let ev = rig.events();
    assert_eq!(
        control_changes(&ev),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
    assert!(ev.iter().any(|e| matches!(
        e,
        AgentEvent::DisplayDetached { reason, .. } if reason == "computer_error"
    )));
    assert_eq!(rig.registry.live_native_views(), 0);
    let view = rig.registry.viewport(false);
    assert_eq!(view.issue, Some(ViewportIssue::ComputerError));
}

#[test]
fn guest_side_shutdown_detaches() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    let cid = rig.cid();
    rig.fake.lock().inject_vm_state(&cid, "stopped");
    rig.registry.pump();
    assert_eq!(rig.registry.live_native_views(), 0);
    assert_eq!(rig.viewport(), ViewportState::Off);
}

// --- control ownership ---

#[test]
fn take_control_requires_a_ready_display() {
    let mut rig = Rig::graphical();
    assert!(matches!(
        rig.registry.take_control(),
        Err(CoreError::ControlUnavailable(_))
    ));
    rig.registry.start().unwrap();
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap();
    rig.guest_ready();
    assert!(
        matches!(
            rig.registry.take_control(),
            Err(CoreError::ControlUnavailable(_))
        ),
        "compositor not ready yet"
    );
    assert!(!rig.display.is_interactive());
    rig.report(GFX_READY);
    rig.events();
    assert_eq!(rig.registry.take_control().unwrap(), ControlOwner::User);
    assert!(rig.display.is_interactive());
    assert_eq!(rig.viewport(), ViewportState::UserControlled);
    assert_eq!(
        control_changes(&rig.events()),
        vec![(ControlOwner::None, ControlOwner::User)]
    );
    // Idempotent: no second event.
    assert_eq!(rig.registry.take_control().unwrap(), ControlOwner::User);
    assert!(control_changes(&rig.events()).is_empty());
}

#[test]
fn return_control_is_idempotent_and_stops_routing_input() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    assert_eq!(rig.registry.return_control().unwrap(), ControlOwner::None);
    assert!(!rig.display.is_interactive());
    assert_eq!(rig.viewport(), ViewportState::Ready);
    assert_eq!(rig.registry.return_control().unwrap(), ControlOwner::None);
    assert_eq!(
        control_changes(&rig.events()),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
}

#[test]
fn control_auto_returns_on_pause() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    rig.registry.pause().unwrap();
    assert_eq!(rig.registry.control_owner(), ControlOwner::None);
    assert!(!rig.display.is_interactive());
    assert_eq!(
        control_changes(&rig.events()),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
    rig.registry.resume().unwrap();
    assert_eq!(
        rig.registry.control_owner(),
        ControlOwner::None,
        "resume never re-grants control"
    );
}

#[test]
fn control_auto_returns_on_stop_before_the_view_goes() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.display.clear_calls();
    rig.events();
    rig.registry.stop().unwrap();
    let calls = rig.display.calls();
    assert_eq!(
        calls.first(),
        Some(&TestDisplayCall::SetInteractive(false)),
        "input released before detach: {calls:?}"
    );
    assert!(matches!(calls.get(1), Some(TestDisplayCall::Detach(_))));
    let ev = rig.events();
    let control_at = ev.iter().position(|e| control_change(e).is_some());
    let detach_at = ev.iter().position(is_detached);
    assert!(control_at < detach_at, "events: {ev:?}");
}

#[test]
fn control_auto_returns_when_the_display_stops_being_ready() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.report(GFX_FAILED);
    assert_eq!(rig.registry.control_owner(), ControlOwner::None);
    // Guest runtime loss also ends control (viewport not Ready).
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    let cid = rig.cid();
    rig.fake.lock().inject_guest_disconnected(&cid, "eof");
    rig.registry.pump();
    assert_eq!(rig.registry.control_owner(), ControlOwner::None);
}

#[test]
fn native_detach_returns_control_and_forgets_the_slot() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    rig.display.state().attachment = pegoles_computer::DisplayAttachment::Detached;
    rig.display.state().live_views = 0;
    rig.display.push_event(DisplayEvent::Detached {
        computer_id: rig.id,
        reason: "window_closed".into(),
    });
    rig.registry.pump();
    let ev = rig.events();
    assert_eq!(count(&ev, is_detached), 1);
    assert_eq!(
        control_changes(&ev),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
    rig.registry.pump();
    assert_eq!(
        rig.attach_calls(),
        1,
        "no auto re-attach after native detach"
    );
    assert_eq!(rig.viewport(), ViewportState::DisplayStarting);
}

#[test]
fn return_control_requested_by_the_human_is_honored() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    rig.display
        .push_event(DisplayEvent::ReturnControlRequested {
            computer_id: rig.id,
        });
    let published = rig.registry.pump();
    assert_eq!(
        control_changes(&published),
        vec![(ControlOwner::User, ControlOwner::None)]
    );
    assert!(!rig.display.is_interactive());
    assert_eq!(rig.viewport(), ViewportState::Ready);
    // A stale request for another computer is ignored.
    rig.registry.take_control().unwrap();
    rig.display
        .push_event(DisplayEvent::ReturnControlRequested {
            computer_id: ComputerId::new(),
        });
    rig.registry.pump();
    assert_eq!(rig.registry.control_owner(), ControlOwner::User);
}

#[test]
fn explicit_detach_returns_control_and_removes_the_view() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    rig.events();
    let published = rig.registry.detach_display().unwrap();
    assert_eq!(count(&published, is_detached), 1);
    assert_eq!(rig.registry.control_owner(), ControlOwner::None);
    assert_eq!(rig.registry.live_native_views(), 0);
    rig.registry.pump();
    assert_eq!(rig.attach_calls(), 1, "explicit detach is not undone");
}

/// The Phase 4 manual-control journey, end to end, asserting the order
/// of the facts a user would see in Activity.
#[test]
fn manual_control_lifecycle_end_to_end() {
    let mut rig = Rig::graphical();
    rig.registry.set_display_geometry(slot(), BOUNDS).unwrap(); // slot mounted early
    rig.registry.start().unwrap();
    rig.guest_ready();
    rig.report(GFX_READY);
    rig.registry.take_control().unwrap();
    let mut resized = slot();
    resized.rect.width = 1000.0;
    resized.rect.height = 625.0;
    resized.animate_ms = 260;
    assert_eq!(
        rig.registry.set_display_geometry(resized, BOUNDS).unwrap(),
        GeometryOutcome::Updated
    );
    assert_eq!(rig.viewport(), ViewportState::UserControlled);
    rig.registry.pause().unwrap();
    rig.registry.resume().unwrap();
    rig.registry.take_control().unwrap();
    rig.registry.stop().unwrap();
    rig.registry.pump();

    let kinds: Vec<&'static str> = rig
        .events()
        .iter()
        .filter_map(|e| match e {
            AgentEvent::GuestRuntimeReady { .. } => Some("guest_ready"),
            AgentEvent::DisplayAttached { .. } => Some("attached"),
            AgentEvent::GraphicalSessionReady { .. } => Some("session_ready"),
            AgentEvent::DisplayReady { .. } => Some("display_ready"),
            AgentEvent::ControlOwnershipChanged {
                to: ControlOwner::User,
                ..
            } => Some("user"),
            AgentEvent::ControlOwnershipChanged {
                to: ControlOwner::None,
                ..
            } => Some("returned"),
            AgentEvent::DisplayDetached { .. } => Some("detached"),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "attached",
            "guest_ready",
            "session_ready",
            "display_ready",
            "user",
            "returned", // pause
            "user",
            "returned", // stop
            "detached",
        ]
    );
    assert_eq!(rig.registry.live_native_views(), 0);
    assert_eq!(rig.registry.control_owner(), ControlOwner::None);
}

// --- default display config ---

#[test]
fn default_display_rules() {
    assert_eq!(
        default_display_config(true, true, true),
        DisplayConfig::for_profile(DisplayProfile::DesktopLarge)
    );
    for (image, backend, adapter) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
        (false, false, false),
    ] {
        assert_eq!(default_display_config(image, backend, adapter), None);
    }
}

fn write_derived_image(images: &Path, graphical: bool) {
    let dir = images.join(pegoles_computer::PEGOLES_BASE_IMAGE_ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("disk.raw"), b"derived").unwrap();
    std::fs::write(dir.join("disk.raw.verified"), "abc").unwrap();
    let mut manifest = serde_json::json!({
        "image_id": pegoles_computer::PEGOLES_BASE_IMAGE_ID,
        "pegoles_image_version": "0.2",
        "debian_version": "13",
        "architecture": "arm64",
        "guest_runtime_version": "0.2.0",
        "guest_protocol_version": 1,
        "source_image_sha512": "src",
        "image_sha512": "abc",
        "built_at": "2026-09-21T00:00:00Z",
        "artifacts": [{
            "file_name": "disk.raw",
            "disk_format": "raw",
            "sha512": "abc",
            "bytes": 7,
            "architecture": "arm64",
            "guest_runtime_version": "0.2.0",
            "source_image_sha512": "src",
            "built_at": "2026-09-21T00:00:00Z",
        }],
    });
    if graphical {
        manifest["graphical"] = serde_json::json!({
            "compositor": "weston 14.0.1",
            "terminal": "foot 1.21.0",
        });
    }
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
}

fn create_default_with(graphical_image: bool, install_display: bool) -> Option<DisplayConfig> {
    let tmp = tempfile::tempdir().unwrap();
    let images = tmp.path().join("images");
    write_derived_image(&images, graphical_image);
    let mut registry =
        ComputerRegistry::with_dirs(EventBus::new(), BackendKind::Mock, tmp.path().to_path_buf());
    if install_display {
        registry.install_display_backend(Box::new(TestDisplay::new()));
    }
    let backend = MacOSVirtualizationBackend::with_transport(
        images,
        tmp.path().join("computers"),
        Box::new(SharedFakeTransport::new()),
    );
    registry.create_default_on(Box::new(backend)).unwrap();
    registry.display_config()
}

#[test]
fn create_default_uses_the_graphical_manifest() {
    assert_eq!(
        create_default_with(true, true),
        DisplayConfig::for_profile(DisplayProfile::DesktopLarge)
    );
    assert_eq!(create_default_with(false, true), None, "headless image");
    assert_eq!(create_default_with(true, false), None, "no display adapter");
}

#[test]
fn backends_without_graphics_refuse_a_display_config() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry =
        ComputerRegistry::with_dirs(EventBus::new(), BackendKind::Mock, tmp.path().to_path_buf());
    let mut cfg = default_config();
    cfg.display = DisplayConfig::for_profile(DisplayProfile::DesktopLarge);
    assert!(matches!(
        registry.create(cfg),
        Err(CoreError::Computer(ComputerError::InvalidConfig(_)))
    ));
    let mut cfg = default_config();
    cfg.display = Some(DisplayConfig::custom(1441, 900));
    let backend = MacOSVirtualizationBackend::with_transport(
        tmp.path().join("images"),
        tmp.path().join("computers"),
        Box::new(SharedFakeTransport::new()),
    );
    assert!(matches!(
        registry.create_on(Box::new(backend), cfg),
        Err(CoreError::Computer(ComputerError::InvalidConfig(_)))
    ));
    // The Mock default path stays headless and creatable.
    registry.create_default().unwrap();
    assert_eq!(registry.display_config(), None);
    assert_eq!(registry.state(), Some(ComputerState::Stopped));
}

#[test]
fn computer_view_wire_shape() {
    let mut rig = Rig::graphical();
    rig.boot_display_ready();
    rig.registry.take_control().unwrap();
    let v = serde_json::to_value(rig.registry.computer_view(false)).unwrap();
    assert_eq!(v["viewport_state"], "user_controlled");
    assert_eq!(v["viewport_issue"], serde_json::Value::Null);
    assert_eq!(v["display_available"], true);
    assert_eq!(
        v["display_config"],
        serde_json::json!({"profile": "desktop_large", "width_px": 1440, "height_px": 900})
    );
    assert_eq!(v["display_backend"], "test");
    assert_eq!(v["display_attached"], true);
    assert_eq!(v["control_owner"], "user");
    assert!(v["display_ready_ms"].is_u64());
    let gs = &v["graphical_session"];
    assert_eq!(gs["state"], "ready");
    assert_eq!(gs["reported"], true);
    assert_eq!(gs["compositor"], "weston");
    assert_eq!(
        (gs["width_px"].as_u64(), gs["height_px"].as_u64()),
        (Some(1440), Some(900))
    );
    for key in ["detail", "since_ms", "ready_in_ms"] {
        assert!(gs.get(key).is_some(), "graphical_session.{key}");
    }
    let outcome = serde_json::to_value(GeometryOutcome::Unchanged).unwrap();
    assert_eq!(outcome, "unchanged");
}

// --- guards ---

/// SECURITY guard (Phase 5): `ControlOwner::Agent` is reachable ONLY
/// through the policy-checked executor (`input.rs` acquire/release +
/// arbiter) and the human-takeover check (`display.rs`). (1) No other
/// production source constructs or compares it; (2) driving every HUMAN
/// control path never yields it.
#[test]
fn agent_control_only_from_executor() {
    let src_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src_dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name == "display_tests.rs" {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let production = src.split("#[cfg(test)]").next().unwrap_or("");
        for (i, line) in production.lines().enumerate() {
            if !line.contains("ControlOwner::Agent") || line.trim_start().starts_with("//") {
                continue;
            }
            let allowed = if name == "viewport.rs" {
                line.contains("ControlOwner::Agent =>")
            } else if name == "input.rs" {
                line.contains("set_control")
                    || line.contains("ControlOwner::Agent =>")
                    || line.contains("== ControlOwner::Agent")
                    || line.contains("ControlOwner::Agent |")
                    || line.contains("agent_may_act")
            } else if name == "display.rs" {
                line.contains("== ControlOwner::Agent")
            } else {
                false
            };
            assert!(
                allowed,
                "{name}:{} references ControlOwner::Agent outside the executor/takeover paths: {line}",
                i + 1
            );
        }
    }

    let mut rig = Rig::graphical();
    let mut seen = vec![rig.registry.control_owner()];
    rig.boot_display_ready();
    seen.push(rig.registry.take_control().unwrap());
    seen.push(rig.registry.control_owner());
    rig.registry.pause().unwrap();
    seen.push(rig.registry.control_owner());
    rig.registry.resume().unwrap();
    seen.push(rig.registry.take_control().unwrap());
    seen.push(rig.registry.return_control().unwrap());
    rig.registry.stop().unwrap();
    seen.push(rig.registry.control_owner());
    assert!(seen.iter().all(|o| *o != ControlOwner::Agent));
    assert!(rig
        .events()
        .iter()
        .filter_map(control_change)
        .all(|(from, to)| from != ControlOwner::Agent && to != ControlOwner::Agent));
}

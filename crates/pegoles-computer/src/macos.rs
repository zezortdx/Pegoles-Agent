//! Real macOS backend: drives Apple Virtualization.framework through the
//! narrow `pegoles-vm-host` helper (child process, JSON Lines on stdio).
//!
//! ```text
//! MacOSVirtualizationBackend --stdin/stdout--> pegoles-vm-host (Swift)
//!                                                  |
//!                                         Virtualization.framework
//!                                                  |
//!                                            Debian 13 ARM64
//! ```
//!
//! SECURITY: this backend only performs VM lifecycle operations. The wire
//! protocol has no shell/file/network commands (see `vmhost_proto.rs` and
//! `docs/VM_HOST_PROTOCOL.md`). Crash/disconnect of the helper surfaces as
//! `BackendDisconnected` and forces the cached state to `Error` so Core
//! never displays a stale Running.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, SnapshotId};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::require_real_backend_supported;
use crate::error::{ComputerError, Result};
use crate::image::PEGOLES_DEBIAN_13_ARM64;
use crate::native_backend::{BackendProfile, HostTransport, NativeHelperBackend};
use crate::platform::{BackendCapabilities, DiskFormat, GuestArchitecture};
use crate::traits::{ComputerBackend, ComputerInstance};

/// Thin platform wrapper: identity, profile, and platform gate live here;
/// every behavior lives in the shared [`NativeHelperBackend`] engine.
pub struct MacOSVirtualizationBackend {
    engine: NativeHelperBackend,
}

fn macos_profile() -> BackendProfile {
    BackendProfile {
        helper_display_name: "pegoles-vm-host",
        helper_env_var: "PEGOLES_VM_HOST",
        dev_helper_relpaths: &["../../native/macos/pegoles-vm-host/.build/release/pegoles-vm-host"],
        resource_helper_name: "pegoles-vm-host",
        official_spec: PEGOLES_DEBIAN_13_ARM64,
        allow_official_fallback: true,
        disk_file_name: "disk.img",
        disk_format: crate::platform::DiskFormat::Raw,
        want_serial_log: true,
        capabilities: BackendCapabilities {
            pause: true,
            resume: true,
            snapshot: false,
            // Phase 4: virtio graphics device + human USB keyboard /
            // absolute pointer are attached when `ComputerConfig.display`
            // is set (headless otherwise). Viewing needs the in-process
            // display adapter of the desktop app.
            graphical_display: true,
            vsock: true,
            dynamic_memory: false,
            guest_arch: GuestArchitecture::Arm64,
            disk_formats: vec![DiskFormat::Raw],
        },
        check_platform: require_real_backend_supported,
    }
}

impl std::fmt::Debug for MacOSVirtualizationBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacOSVirtualizationBackend")
            .field("engine", &self.engine)
            .finish()
    }
}

impl MacOSVirtualizationBackend {
    /// Production constructor. Refuses unsupported platforms immediately:
    /// a real VM must never be attempted where it cannot run, and must
    /// never degrade into host execution.
    pub fn new(images_dir: PathBuf, computers_dir: PathBuf) -> Result<Self> {
        (macos_profile().check_platform)()?;
        Ok(Self {
            engine: NativeHelperBackend::with_profile(images_dir, computers_dir, macos_profile()),
        })
    }

    /// Test injection: pre-opened transport instead of spawning a process.
    /// Skips the platform gate (no real VM is involved); production code
    /// must use `new()`.
    pub fn with_transport(
        images_dir: PathBuf,
        computers_dir: PathBuf,
        transport: Box<dyn HostTransport>,
    ) -> Self {
        Self {
            engine: NativeHelperBackend::with_transport(
                images_dir,
                computers_dir,
                macos_profile(),
                transport,
            ),
        }
    }

    pub fn serial_log_path(&self) -> Option<PathBuf> {
        self.engine.serial_log_path()
    }
}

impl ComputerBackend for MacOSVirtualizationBackend {
    fn create(&mut self, config: ComputerConfig) -> Result<ComputerId> {
        self.engine.backend_create(config)
    }
    fn start(&mut self) -> Result<ComputerState> {
        self.engine.backend_start()
    }
    fn stop(&mut self) -> Result<ComputerState> {
        self.engine.backend_stop()
    }
    fn pause(&mut self) -> Result<ComputerState> {
        self.engine.backend_pause()
    }
    fn resume(&mut self) -> Result<ComputerState> {
        self.engine.backend_resume()
    }
    fn reset(&mut self) -> Result<ComputerState> {
        self.engine.backend_reset()
    }
    fn destroy(&mut self) -> Result<()> {
        self.engine.backend_destroy()
    }
    fn snapshot(&mut self) -> Result<SnapshotId> {
        Err(ComputerError::UnsupportedOperation(
            "snapshots are not implemented for the real VM backend".to_string(),
        ))
    }
    fn restore(&mut self, _id: SnapshotId) -> Result<ComputerState> {
        Err(ComputerError::UnsupportedOperation(
            "snapshots are not implemented for the real VM backend".to_string(),
        ))
    }
    fn state(&self) -> ComputerState {
        self.engine.backend_state()
    }
    fn id(&self) -> Option<ComputerId> {
        self.engine.computer_id()
    }
    fn config(&self) -> Option<ComputerConfig> {
        self.engine.backend_config()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.engine.backend_capabilities()
    }
    fn instance(&self) -> Option<ComputerInstance> {
        self.engine.backend_instance()
    }
    fn serial_log_path(&self) -> Option<PathBuf> {
        self.engine.serial_log_path()
    }
    fn guest_state(&self) -> pegoles_protocol::GuestRuntimeState {
        self.engine.guest_state()
    }
    fn guest_info(&self) -> Option<pegoles_guest_proto::SystemInfo> {
        self.engine.guest_info()
    }
    fn guest_ready_ms(&self) -> Option<u64> {
        self.engine.guest_ready_ms()
    }
    fn capability_diagnostics(&self) -> Vec<crate::CapabilityDiagnostic> {
        self.engine.capability_diagnostics()
    }
    fn graphical_session(&self) -> crate::guest::GraphicalSessionInfo {
        self.engine.graphical_session()
    }
    fn poll_guest(&mut self) -> Vec<crate::guest::GuestObservation> {
        self.engine.poll_guest()
    }
    fn guest_ping(&mut self, timeout: Duration) -> Result<u64> {
        self.engine.guest_ping(timeout)
    }
    fn guest_info_request(&mut self, timeout: Duration) -> Result<pegoles_guest_proto::SystemInfo> {
        self.engine.guest_info_request(timeout)
    }
    fn input_available(&self) -> bool {
        self.engine.input_available()
    }
    fn input_capabilities(&self) -> crate::input::InputCapabilities {
        self.engine.input_capabilities()
    }
    fn input_execute(
        &mut self,
        request_id: &str,
        op: &crate::input::InputOp,
    ) -> crate::input::InputOutcome {
        self.engine.input_execute(request_id, op)
    }
    fn input_capture_frame(
        &mut self,
        request_id: &str,
        timeout: Duration,
    ) -> Result<crate::input::CapturedFrame> {
        self.engine.input_capture_frame(request_id, timeout)
    }
}

impl crate::transport::GuestTransport for MacOSVirtualizationBackend {
    fn send_frame(&mut self, payload: &str) -> Result<()> {
        self.engine.transport_send_frame(payload)
    }
    fn poll_events(&mut self) -> Vec<crate::transport::TransportEvent> {
        self.engine.transport_poll_events()
    }
    fn close(&mut self) {
        self.engine.transport_close()
    }
    fn is_connected(&self) -> bool {
        self.engine.transport_is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest::GuestObservation;
    use crate::native_backend::{FakeTransport, HostTransport};
    use std::path::{Path, PathBuf};

    fn test_dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let images = tmp.path().join("images");
        let computers = tmp.path().join("computers");
        (tmp, images, computers)
    }

    fn seed_ready_image(images_dir: &Path) {
        let dir = images_dir.join("pegoles-debian-13-arm64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base.raw"), b"fake-disk-bytes").unwrap();
        std::fs::write(dir.join("base.raw.verified"), "abc123").unwrap();
    }

    #[test]
    fn production_constructor_gates_platform() {
        let (_tmp, images, computers) = test_dirs();
        let result = MacOSVirtualizationBackend::new(images, computers);
        if crate::config::is_real_backend_supported() {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(ComputerError::UnsupportedPlatform(_))));
        }
    }

    #[test]
    fn full_lifecycle_against_fake_host() {
        let (_tmp, images, computers) = test_dirs();
        seed_ready_image(&images);
        let transport: Box<dyn HostTransport> = Box::new(FakeTransport::new());
        let mut b = MacOSVirtualizationBackend::with_transport(images, computers, transport);
        let id = b.create(default_config_for_test()).unwrap();
        assert!(b.id() == Some(id));
        assert_eq!(b.state(), ComputerState::Stopped);
        assert_eq!(b.start().unwrap(), ComputerState::Running);
        assert_eq!(b.state(), ComputerState::Running);
        assert_eq!(b.pause().unwrap(), ComputerState::Paused);
        assert_eq!(b.resume().unwrap(), ComputerState::Running);
        assert_eq!(b.stop().unwrap(), ComputerState::Stopped);
    }

    #[test]
    fn create_writes_layout_and_copies_disk() {
        let (_tmp, images, computers) = test_dirs();
        seed_ready_image(&images);
        let mut b = MacOSVirtualizationBackend::with_transport(
            images.clone(),
            computers.clone(),
            Box::new(FakeTransport::new()),
        );
        let id = b.create(default_config_for_test()).unwrap();
        let dir = computers.join(id.to_string());
        assert!(dir.join("disk.img").is_file());
        assert!(dir.join("metadata.json").is_file());
        assert_eq!(
            std::fs::read(dir.join("disk.img")).unwrap(),
            b"fake-disk-bytes"
        );
        // base untouched
        assert_eq!(
            std::fs::read(images.join("pegoles-debian-13-arm64").join("base.raw")).unwrap(),
            b"fake-disk-bytes"
        );
    }

    #[test]
    fn create_without_image_fails_closed() {
        let (_tmp, images, computers) = test_dirs();
        let mut b = MacOSVirtualizationBackend::with_transport(
            images,
            computers,
            Box::new(FakeTransport::new()),
        );
        let err = b.create(default_config_for_test()).unwrap_err();
        assert!(matches!(err, ComputerError::ImageMissing(_)));
    }

    #[test]
    fn host_crash_surfaces_disconnect_and_error_state() {
        let (_tmp, images, computers) = test_dirs();
        seed_ready_image(&images);
        let mut fake = FakeTransport::new();
        fake.fail_after_sends = Some(1); // dies right after create
        let mut b = MacOSVirtualizationBackend::with_transport(images, computers, Box::new(fake));
        b.create(default_config_for_test()).unwrap();
        let err = b.start().unwrap_err();
        assert!(
            matches!(err, ComputerError::BackendDisconnected(_)),
            "got {err:?}"
        );
        // never a stale Running
        assert_eq!(b.engine.backend_state(), ComputerState::Error);
    }

    #[test]
    fn snapshots_are_explicitly_unsupported() {
        let (_tmp, images, computers) = test_dirs();
        let mut b = MacOSVirtualizationBackend::with_transport(
            images,
            computers,
            Box::new(FakeTransport::new()),
        );
        assert!(matches!(
            b.snapshot(),
            Err(ComputerError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn ops_before_create_fail() {
        let (_tmp, images, computers) = test_dirs();
        let mut b = MacOSVirtualizationBackend::with_transport(
            images,
            computers,
            Box::new(FakeTransport::new()),
        );
        assert_eq!(b.start().unwrap_err(), ComputerError::NotCreated);
    }

    #[test]
    fn state_sync_consumes_async_events() {
        // FakeTransport queues a vm_state_changed event on start; the next
        // state() must reflect native truth, not a stale cache.
        let (_tmp, images, computers) = test_dirs();
        seed_ready_image(&images);
        let mut b = MacOSVirtualizationBackend::with_transport(
            images,
            computers,
            Box::new(FakeTransport::new()),
        );
        b.create(default_config_for_test()).unwrap();
        b.start().unwrap();
        assert_eq!(b.state(), ComputerState::Running);
    }

    fn default_config_for_test() -> ComputerConfig {
        crate::config::default_config()
    }

    /// Shared fake: tests hold one handle, the backend owns the other.
    type SharedFake = crate::native_backend::SharedFakeTransport;

    fn guest_hello_frame() -> String {
        r#"{"type":"guest_hello","protocol_version":1,"runtime_version":"0.1.0","os":"debian","os_version":"13","arch":"aarch64"}"#.to_string()
    }

    fn ready_backend() -> (
        tempfile::TempDir,
        MacOSVirtualizationBackend,
        ComputerId,
        SharedFake,
    ) {
        let (tmp, images, computers) = test_dirs();
        seed_ready_image(&images);
        let fake = SharedFake::default();
        let mut b =
            MacOSVirtualizationBackend::with_transport(images, computers, Box::new(fake.clone()));
        let id = b.create(default_config_for_test()).unwrap();
        b.start().unwrap();
        assert_eq!(
            b.guest_state(),
            pegoles_protocol::GuestRuntimeState::Waiting
        );
        (tmp, b, id, fake)
    }

    fn connect_handshake(fake: &SharedFake, cid: &str) {
        let mut f = fake.lock();
        f.inject_guest_connected(cid);
        f.inject_guest_frame(cid, &guest_hello_frame());
        f.inject_guest_frame(cid, r#"{"type":"ready"}"#);
    }

    #[test]
    fn guest_handshake_to_ready_via_poll() {
        let (_tmp, mut b, id, fake) = ready_backend();
        let cid = id.to_string();
        connect_handshake(&fake, &cid);
        let obs = b.poll_guest();
        assert_eq!(b.guest_state(), pegoles_protocol::GuestRuntimeState::Ready);
        assert!(obs
            .iter()
            .any(|o| matches!(o, GuestObservation::BecameReady { .. })));
        assert!(b.guest_ready_ms().is_some());
        // HostHello was actually sent over the (fake) wire.
        let sent = fake.lock().sent.join("\n");
        assert!(sent.contains("host_hello"));
    }

    #[test]
    fn guest_ping_round_trip_reports_latency() {
        let (_tmp, mut b, id, fake) = ready_backend();
        connect_handshake(&fake, &id.to_string());
        assert!(b
            .poll_guest()
            .iter()
            .any(|o| matches!(o, GuestObservation::BecameReady { .. })));
        // FakeTransport auto-answers Ping with a Pong echo.
        let latency = b
            .guest_ping(Duration::from_secs(5))
            .expect("ping round-trip");
        assert!(latency < 5000);
    }

    #[test]
    fn guest_info_request_returns_system_info() {
        let (_tmp, mut b, id, fake) = ready_backend();
        connect_handshake(&fake, &id.to_string());
        let _ = b.poll_guest();
        let info = b
            .guest_info_request(Duration::from_secs(5))
            .expect("system info");
        assert_eq!(info.os, "debian");
        assert_eq!(info.arch, "aarch64");
        assert_eq!(b.guest_info().map(|i| i.os), Some("debian".to_string()));
    }

    #[test]
    fn guest_disconnect_moves_out_of_ready() {
        let (_tmp, mut b, id, fake) = ready_backend();
        let cid = id.to_string();
        connect_handshake(&fake, &cid);
        let _ = b.poll_guest();
        assert_eq!(b.guest_state(), pegoles_protocol::GuestRuntimeState::Ready);
        fake.lock().inject_guest_disconnected(&cid, "eof");
        let obs = b.poll_guest();
        assert_eq!(
            b.guest_state(),
            pegoles_protocol::GuestRuntimeState::Disconnected
        );
        assert!(obs.iter().any(|o| matches!(
            o,
            GuestObservation::StateChanged {
                to: pegoles_protocol::GuestRuntimeState::Disconnected,
                ..
            }
        )));
        // Reconnect recovers without a VM reboot.
        connect_handshake(&fake, &cid);
        let _ = b.poll_guest();
        assert_eq!(b.guest_state(), pegoles_protocol::GuestRuntimeState::Ready);
    }

    #[test]
    fn incompatible_guest_never_proceeds_silently() {
        let (_tmp, mut b, id, fake) = ready_backend();
        let cid = id.to_string();
        {
            let mut f = fake.lock();
            f.inject_guest_connected(&cid);
            f.inject_guest_frame(
                &cid,
                r#"{"type":"guest_hello","protocol_version":999,"runtime_version":"9.9","os":"debian","os_version":"13","arch":"aarch64"}"#,
            );
        }
        let _ = b.poll_guest();
        assert_eq!(
            b.guest_state(),
            pegoles_protocol::GuestRuntimeState::Incompatible
        );
    }

    const GFX_READY: &str = r#"{"type":"graphical_session","status":"ready","compositor":"weston","width_px":1440,"height_px":900}"#;

    fn gfx_changes(obs: &[GuestObservation]) -> Vec<crate::guest::GraphicalSessionChange> {
        obs.iter()
            .filter_map(|o| match o {
                GuestObservation::GraphicalSessionChanged(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn capabilities_advertise_graphical_display() {
        let (_tmp, b, _id, _fake) = ready_backend();
        assert!(b.capabilities().graphical_display);
    }

    #[test]
    fn graphical_session_requested_after_ready_and_tracked() {
        let (_tmp, mut b, id, fake) = ready_backend();
        fake.lock().graphical_session_reply = Some(GFX_READY.to_string());
        connect_handshake(&fake, &id.to_string());
        let first = b.poll_guest();
        assert!(first
            .iter()
            .any(|o| matches!(o, GuestObservation::BecameReady { .. })));
        // The request went over the wire exactly once...
        let sent = fake.lock().sent.join("\n");
        assert_eq!(sent.matches("get_graphical_session").count(), 1);
        // ...and the guest's answer surfaces on a later poll.
        let mut all = first;
        all.extend(b.poll_guest());
        let changes = gfx_changes(&all);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].to,
            pegoles_protocol::GraphicalSessionState::Ready
        );
        let info = b.graphical_session();
        assert_eq!(info.state, pegoles_protocol::GraphicalSessionState::Ready);
        assert_eq!(info.compositor.as_deref(), Some("weston"));
        // VM stop resets it.
        b.stop().unwrap();
        assert!(!b.graphical_session().reported);
    }

    #[test]
    fn observations_during_blocking_calls_are_not_lost() {
        let (_tmp, mut b, id, fake) = ready_backend();
        let cid = id.to_string();
        connect_handshake(&fake, &cid);
        let _ = b.poll_guest();
        // A pushed report arrives while a ping round-trip is in flight:
        // the ping consumes it, but Core must still see it on next poll.
        fake.lock().inject_guest_frame(&cid, GFX_READY);
        b.guest_ping(Duration::from_secs(5)).expect("ping");
        let changes = gfx_changes(&b.poll_guest());
        assert_eq!(changes.len(), 1, "report swallowed by the ping wait");
    }

    #[test]
    fn guest_ping_before_ready_fails() {
        let (_tmp, mut b, _id, _fake) = ready_backend();
        assert!(matches!(
            b.guest_ping(Duration::from_secs(1)),
            Err(ComputerError::GuestUnavailable(_))
        ));
    }

    /// Real-hardware guest e2e smoke test. Skipped unless
    /// PEGOLES_REAL_GUEST_TEST=1. Requires: macOS arm64, helper built +
    /// signed, DERIVED image Ready (scripts/build-guest-image). Exercises
    /// the full host<->guest control plane: vsock connect, versioned
    /// handshake, Ready, Ping/Pong, SystemInfo, stop. Never runs in CI.
    #[test]
    fn real_guest_e2e_smoke() {
        if std::env::var("PEGOLES_REAL_GUEST_TEST").as_deref() != Ok("1") {
            eprintln!("skipping real guest e2e (set PEGOLES_REAL_GUEST_TEST=1)");
            return;
        }
        assert!(
            crate::config::is_real_backend_supported(),
            "smoke test requires macOS on Apple Silicon"
        );
        let data = crate::config::pegoles_data_dir();
        let mgr = crate::image::ComputerImageManager::new(data.join("images"));
        assert_eq!(
            mgr.derived_status(),
            crate::image::ImageStatus::Ready,
            "build the derived image first (scripts/build-guest-image/build.sh)"
        );

        let mut backend =
            MacOSVirtualizationBackend::new(data.join("images"), data.join("computers"))
                .expect("backend constructor");
        let t0 = std::time::Instant::now();
        let id = backend
            .create(crate::config::default_config())
            .expect("create");
        eprintln!("create took {:?}", t0.elapsed());

        let t1 = std::time::Instant::now();
        assert_eq!(backend.start().expect("start"), ComputerState::Running);
        eprintln!("vm start -> Running: {:?}", t1.elapsed());

        // Wait for the guest handshake (guest boots, systemd starts the
        // runtime, vsock connect, hello, ready). Progress is logged so a
        // failure always shows the computer id and the last guest state.
        eprintln!("waiting for guest handshake (computer {id})");
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        loop {
            let _ = backend.poll_guest();
            if backend.guest_state() == pegoles_protocol::GuestRuntimeState::Ready {
                break;
            }
            if matches!(
                backend.guest_state(),
                pegoles_protocol::GuestRuntimeState::Incompatible
                    | pegoles_protocol::GuestRuntimeState::Error
            ) {
                panic!("guest failed: {:?} (computer {id})", backend.guest_state());
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "guest never became ready (state {:?}, computer {id})",
                    backend.guest_state()
                );
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        let ready_ms = backend.guest_ready_ms().expect("ready duration measured");
        eprintln!("guest connected+handshake -> Ready in {ready_ms} ms");

        let ping = backend
            .guest_ping(Duration::from_secs(15))
            .expect("ping round-trip");
        eprintln!("ping -> pong: {ping} ms");

        let info = backend
            .guest_info_request(Duration::from_secs(15))
            .expect("system info");
        eprintln!(
            "system info: {} {} kernel={} arch={} host={} runtime={} proto={}",
            info.os,
            info.os_version,
            info.kernel,
            info.arch,
            info.hostname,
            info.runtime_version,
            info.protocol_version
        );
        assert_eq!(info.os, "debian");
        assert_eq!(info.os_version, "13");
        assert_eq!(info.arch, "aarch64");
        assert_eq!(info.protocol_version, 1);

        assert_eq!(backend.stop().expect("stop"), ComputerState::Stopped);
        eprintln!("stop -> Stopped: ok (computer {id} kept for inspection)");
    }

    /// Real-hardware smoke test. Skipped unless PEGOLES_REAL_VM_TEST=1.
    /// Requires: macOS arm64, helper built, base image prepared, and the
    /// helper binary signed with the virtualization entitlement.
    /// Never runs in normal CI.
    #[test]
    fn real_vm_lifecycle_smoke() {
        if std::env::var("PEGOLES_REAL_VM_TEST").as_deref() != Ok("1") {
            eprintln!("skipping real VM smoke test (set PEGOLES_REAL_VM_TEST=1)");
            return;
        }
        assert!(
            crate::config::is_real_backend_supported(),
            "smoke test requires macOS on Apple Silicon"
        );
        let data = crate::config::pegoles_data_dir();
        let images = data.join("images");
        let computers = data.join("computers");
        let mgr = crate::image::ComputerImageManager::new(images.clone());
        assert_eq!(
            mgr.status(),
            crate::image::ImageStatus::Ready,
            "prepare the base image first (Prepare Computer in the UI)"
        );

        let mut backend =
            MacOSVirtualizationBackend::new(images, computers).expect("backend constructor");
        let t0 = std::time::Instant::now();
        let id = backend
            .create(crate::config::default_config())
            .expect("create");
        eprintln!("create+validate took {:?}", t0.elapsed());

        let t1 = std::time::Instant::now();
        let state = backend.start().expect("start");
        eprintln!("start -> {state:?} took {:?}", t1.elapsed());
        assert_eq!(state, ComputerState::Running);
        assert_eq!(backend.state(), ComputerState::Running);

        // Let the guest boot while serial accumulates.
        std::thread::sleep(std::time::Duration::from_secs(30));
        let serial = backend.serial_log_path().expect("serial path");
        assert!(serial.is_file(), "serial log file must exist");
        let bytes = std::fs::metadata(&serial).map(|m| m.len()).unwrap_or(0);
        eprintln!("serial.log: {bytes} bytes at {}", serial.display());

        assert_eq!(backend.pause().expect("pause"), ComputerState::Paused);
        assert_eq!(backend.resume().expect("resume"), ComputerState::Running);
        assert_eq!(backend.stop().expect("stop"), ComputerState::Stopped);
        eprintln!("smoke OK; computer dir kept for inspection: {id}");
    }
}

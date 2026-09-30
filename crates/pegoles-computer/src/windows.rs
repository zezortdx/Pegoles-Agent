//! Windows host: readiness for Pegoles' computer, the one-time setup
//! steps, and the HCS backend wrapper.
//!
//! Architecture (docs/WINDOWS_ARCHITECTURE.md): Host Compute System on the
//! Virtual Machine Platform feature (every Windows 11 edition, Home
//! included), driven by a small LocalSystem broker service because HCS
//! only serves administrators; the unprivileged `pegoles-vm-host.exe`
//! helper talks to the broker for lifecycle and connects to the guest
//! runtime directly over AF_HYPERV. Nothing here shells out to PowerShell,
//! WMI or GUI tools.
//!
//! Readiness is detected with documented OS calls only; anything a call
//! cannot prove stays unknown and is never reported as ready.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, SnapshotId};
use serde::{Deserialize, Serialize};

use crate::error::{ComputerError, Result};
use crate::platform::{BackendCapabilities, DiskFormat, GuestArchitecture};
use crate::traits::ComputerBackend;

/// Well-known VSOCK template GUID from Microsoft's "Make your own
/// integration services" doc: Data1 carries the Linux guest port.
pub const HV_VSOCK_TEMPLATE_SUFFIX: &str = "facb-11e6-bd58-64006a7986d3";

/// Deterministic Hyper-V service GUID for a Linux-guest VSOCK port.
/// Single source of truth — never hardcode GUIDs elsewhere.
pub fn hyperv_service_guid_for_port(port: u32) -> String {
    format!("{port:08X}-{HV_VSOCK_TEMPLATE_SUFFIX}")
}

/// The guest runtime's listener on Hyper-V (the host connects to it).
pub fn pegoles_hyperv_service_guid() -> String {
    hyperv_service_guid_for_port(pegoles_guest_proto::PEGOLES_GUEST_LISTEN_PORT)
}

/// First Windows 11 build.
pub const WINDOWS_11_BUILD: u32 = 22000;

/// Whether this PC can run Pegoles' computer right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualizationReadiness {
    Ready,
    /// Virtual Machine Platform is off (Pegoles can turn it on, elevated).
    NeedsEnable,
    /// Turned on; Windows must restart once.
    RestartPending,
    /// Intel VT-x / AMD-V is off in the firmware (only the person can
    /// change it).
    FirmwareDisabled,
    /// This processor or architecture cannot run it.
    Unsupported,
    /// Could not be determined.
    Unknown,
}

/// Raw facts, each proven by an OS call or `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadinessFacts {
    pub build: Option<u32>,
    pub x64: bool,
    /// A hypervisor runs under Windows (CPUID leaf 1, ECX bit 31).
    pub hypervisor_present: bool,
    /// The processor supports VT-x (Intel) or AMD-V (SVM).
    pub cpu_supports_vt: Option<bool>,
    /// Firmware exposes virtualization (only meaningful with no hypervisor).
    pub firmware_vt: Option<bool>,
    /// The Host Compute Service (installed with Virtual Machine Platform).
    pub vmcompute_installed: Option<bool>,
    pub reboot_pending: Option<bool>,
    /// Pegoles' broker service is registered.
    pub broker_installed: Option<bool>,
}

/// What onboarding and the capability report use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WindowsReadiness {
    /// "Windows 11 Home (24H2)".
    pub os_name: String,
    pub os_supported: bool,
    pub state: VirtualizationReadiness,
    /// Pegoles can fix it itself (elevated, with consent).
    pub fixable: bool,
    pub broker_installed: bool,
    /// Facts for "Technical details".
    pub technical: String,
}

/// Pure classification (unit-tested on every OS). Order: architecture,
/// what already works, then what blocks, then what Pegoles can fix.
pub fn classify(f: &ReadinessFacts) -> (VirtualizationReadiness, bool) {
    use VirtualizationReadiness as V;
    if !f.x64 {
        return (V::Unsupported, false);
    }
    match f.vmcompute_installed {
        Some(true) if f.hypervisor_present => (V::Ready, false),
        Some(true) => {
            if f.reboot_pending == Some(true) {
                (V::RestartPending, false)
            } else if f.firmware_vt == Some(false) {
                (V::FirmwareDisabled, false)
            } else {
                // Feature on, hypervisor not launched, nothing pending:
                // unusual (hypervisor launch disabled); say so honestly.
                (V::Unknown, false)
            }
        }
        Some(false) => {
            if f.cpu_supports_vt == Some(false) {
                (V::Unsupported, false)
            } else if !f.hypervisor_present && f.firmware_vt == Some(false) {
                (V::FirmwareDisabled, false)
            } else if f.reboot_pending == Some(true) {
                (V::RestartPending, false)
            } else {
                (V::NeedsEnable, true)
            }
        }
        None => (V::Unknown, false),
    }
}

/// "Windows 11 Home (24H2)" from registry facts. The registry's
/// `ProductName` still says "Windows 10" on Windows 11, so the name comes
/// from the build number and the edition.
pub fn os_name(build: Option<u32>, edition: Option<&str>, display_version: Option<&str>) -> String {
    let family = match build {
        Some(b) if b >= WINDOWS_11_BUILD => "Windows 11",
        Some(_) => "Windows 10",
        None => "Windows",
    };
    let edition = match edition.map(str::to_ascii_lowercase).as_deref() {
        Some("core" | "coresinglelanguage" | "corecountryspecific" | "coren") => Some("Home"),
        Some("professional" | "professionaln") => Some("Pro"),
        Some("professionalworkstation") => Some("Pro for Workstations"),
        Some("professionaleducation") => Some("Pro Education"),
        Some("enterprise" | "enterprisen" | "enterprises") => Some("Enterprise"),
        Some("education" | "educationn") => Some("Education"),
        Some("serverstandard" | "serverdatacenter" | "serverdatacenterazureedition") => {
            Some("Server")
        }
        _ => None,
    };
    let mut name = family.to_string();
    if let Some(edition) = edition {
        name = if edition == "Server" {
            "Windows Server".to_string()
        } else {
            format!("{name} {edition}")
        };
    }
    if let Some(v) = display_version.filter(|v| !v.is_empty() && v.len() <= 8) {
        name = format!("{name} ({v})");
    }
    name
}

pub fn readiness_from(
    facts: &ReadinessFacts,
    edition: Option<&str>,
    display_version: Option<&str>,
) -> WindowsReadiness {
    let (mut state, mut fixable) = classify(facts);
    let broker_installed = facts.broker_installed == Some(true);
    if state == VirtualizationReadiness::Ready && facts.broker_installed == Some(false) {
        // The PC is ready but Pegoles' own service is missing: a broken
        // install, not something to "turn on".
        state = VirtualizationReadiness::Unknown;
        fixable = false;
    }
    let yn = |v: Option<bool>| match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    };
    let technical = format!(
        "build {}; x64 {}; hypervisor running {}; CPU virtualization {}; firmware virtualization {}; \
         Virtual Machine Platform (vmcompute) {}; restart pending {}; Pegoles VM broker service {}",
        facts.build.map_or("unknown".into(), |b| b.to_string()),
        yn(Some(facts.x64)),
        yn(Some(facts.hypervisor_present)),
        yn(facts.cpu_supports_vt),
        yn(facts.firmware_vt),
        yn(facts.vmcompute_installed),
        yn(facts.reboot_pending),
        yn(facts.broker_installed),
    );
    WindowsReadiness {
        os_name: os_name(facts.build, edition, display_version),
        os_supported: facts.build.is_some_and(|b| b >= WINDOWS_11_BUILD) && facts.x64,
        state,
        fixable,
        broker_installed,
        technical,
    }
}

/// This PC, as far as Windows says. Off Windows: `Unsupported`.
pub fn readiness() -> WindowsReadiness {
    #[cfg(target_os = "windows")]
    {
        detect::readiness()
    }
    #[cfg(not(target_os = "windows"))]
    {
        readiness_from(&ReadinessFacts::default(), None, None)
    }
}

/// Result of asking Windows to turn virtualization on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnableOutcome {
    Enabled,
    RestartRequired,
    /// The person said no to the administrator prompt.
    Declined,
}

/// Run `pegoles-broker.exe enable-virtualization` elevated (Windows shows
/// its own administrator prompt; onboarding explains why first) and wait.
pub fn enable_virtualization_elevated() -> Result<EnableOutcome> {
    #[cfg(target_os = "windows")]
    {
        detect::enable_virtualization_elevated()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(ComputerError::UnsupportedPlatform(
            "turning on virtualization is a Windows step".into(),
        ))
    }
}

/// Restart Windows now (only after the person pressed "Restart now") and
/// open Pegoles once after they sign back in.
pub fn restart_for_setup() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        detect::restart_for_setup()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(ComputerError::UnsupportedPlatform(
            "a restart is only needed on Windows".into(),
        ))
    }
}

/// Remove the one-time "open Pegoles after the restart" entry (called at
/// startup; harmless when absent).
pub fn clear_resume_after_restart() {
    #[cfg(target_os = "windows")]
    detect::clear_resume_after_restart();
}

#[cfg(target_os = "windows")]
mod detect;

/// Real Windows backend: persistent `ComputerId` identity with ephemeral
/// HCS compute systems per start (see `ComputerInstance`). Thin wrapper
/// like its macOS sibling: profile + platform gate here, all behavior in
/// the shared [`NativeHelperBackend`] engine, driven through
/// `pegoles-vm-host.exe` over the same JSONL command set.
#[derive(Debug)]
pub struct WindowsHcsBackend {
    engine: crate::native_backend::NativeHelperBackend,
}

fn windows_profile() -> crate::native_backend::BackendProfile {
    use crate::native_backend::BackendProfile;
    BackendProfile {
        helper_display_name: "pegoles-vm-host.exe",
        helper_env_var: "PEGOLES_VM_HOST_WINDOWS",
        dev_helper_relpaths: &[
            "../../target/release/pegoles-vm-host.exe",
            "../../target/debug/pegoles-vm-host.exe",
        ],
        resource_helper_name: "pegoles-vm-host.exe",
        // No official RAW fallback on Windows: only the derived VHDX boots
        // on Hyper-V. Missing derived image fails closed as ImageMissing.
        official_spec: crate::image::GENERIC_DEBIAN_13_ARM64,
        allow_official_fallback: false,
        disk_file_name: "disk.vhdx",
        disk_format: DiskFormat::Vhdx,
        // COM1 → the helper's pipe → logs/serial.log (bounded).
        want_serial_log: true,
        capabilities: BackendCapabilities {
            pause: true,
            resume: true,
            snapshot: false,
            graphical_display: false,
            vsock: true,
            dynamic_memory: false,
            guest_arch: GuestArchitecture::X86_64,
            disk_formats: vec![DiskFormat::Vhdx],
        },
        check_platform: check_windows_platform,
    }
}

/// Platform gate: Windows x64 with virtualization ready and Pegoles'
/// broker installed. Never a silent Mock fallback.
pub fn check_windows_platform() -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        Err(ComputerError::UnsupportedPlatform(
            "WindowsHcsBackend runs on Windows only".to_string(),
        ))
    }
    #[cfg(target_os = "windows")]
    {
        let r = readiness();
        match r.state {
            VirtualizationReadiness::Ready if r.broker_installed => Ok(()),
            VirtualizationReadiness::Ready => Err(ComputerError::SetupRequired(
                "Pegoles' VM service is not installed; install Pegoles again".into(),
            )),
            VirtualizationReadiness::Unsupported => {
                Err(ComputerError::UnsupportedPlatform(r.technical))
            }
            _ => Err(ComputerError::SetupRequired(format!(
                "virtualization is not ready ({:?}): {}",
                r.state, r.technical
            ))),
        }
    }
}

impl WindowsHcsBackend {
    /// Validates platform + host support, then owns a shared engine.
    /// Test injection goes through `with_transport` (no live HCS needed).
    pub fn new(images_dir: std::path::PathBuf, computers_dir: std::path::PathBuf) -> Result<Self> {
        (windows_profile().check_platform)()?;
        Ok(Self {
            engine: crate::native_backend::NativeHelperBackend::with_profile(
                images_dir,
                computers_dir,
                windows_profile(),
            ),
        })
    }

    /// Test injection: pre-opened transport instead of spawning a process.
    pub fn with_transport(
        images_dir: std::path::PathBuf,
        computers_dir: std::path::PathBuf,
        transport: Box<dyn crate::native_backend::HostTransport>,
    ) -> Self {
        Self {
            engine: crate::native_backend::NativeHelperBackend::with_transport(
                images_dir,
                computers_dir,
                windows_profile(),
                transport,
            ),
        }
    }
}

impl ComputerBackend for WindowsHcsBackend {
    fn create(&mut self, config: ComputerConfig) -> Result<ComputerId> {
        // Windows validates as amd64 guest: the shared config bounds apply,
        // and the derived VHDX requirement is enforced by the engine
        // profile (no official RAW fallback).
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
        Err(ComputerError::BackendFeatureNotImplemented(
            "snapshots are not implemented for the Windows backend".to_string(),
        ))
    }
    fn restore(&mut self, _id: SnapshotId) -> Result<ComputerState> {
        Err(ComputerError::BackendFeatureNotImplemented(
            "snapshots are not implemented for the Windows backend".to_string(),
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
    fn instance(&self) -> Option<crate::traits::ComputerInstance> {
        self.engine.backend_instance()
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
    fn guest_ping(&mut self, timeout: std::time::Duration) -> Result<u64> {
        self.engine.guest_ping(timeout)
    }
    fn guest_info_request(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<pegoles_guest_proto::SystemInfo> {
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
    fn input_execute_cancellable(
        &mut self,
        request_id: &str,
        op: &crate::input::InputOp,
        cancelled: &dyn Fn() -> bool,
    ) -> crate::input::InputOutcome {
        self.engine
            .input_execute_cancellable(request_id, op, cancelled)
    }
    fn input_capture_frame(
        &mut self,
        request_id: &str,
        timeout: std::time::Duration,
    ) -> Result<crate::input::CapturedFrame> {
        self.engine.input_capture_frame(request_id, timeout)
    }
    fn input_capture_frame_cancellable(
        &mut self,
        request_id: &str,
        timeout: std::time::Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<crate::input::CapturedFrame> {
        self.engine
            .input_capture_frame_cancellable(request_id, timeout, cancelled)
    }
    fn open_egress(&mut self) -> Result<crate::egress::EgressEndpoint> {
        self.engine.backend_open_egress()
    }
    fn begin_open_egress(&mut self) -> Result<Box<dyn crate::egress::PendingEgressOpen>> {
        self.engine.backend_begin_open_egress()
    }
    fn close_egress(&mut self) -> Result<()> {
        self.engine.backend_close_egress()
    }
}

impl crate::transport::GuestTransport for WindowsHcsBackend {
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

/// Pegoles never automates Hyper-V through shell-outs or GUI scripting.
/// HCS offers a proper API; this test locks the direction by forbidding
/// executable automation patterns (product names in prose are fine).
/// The check scans everything above this test module, so the list
/// itself never matches.
#[cfg(test)]
mod direction_tests {
    #[test]
    fn no_shell_out_automation_in_windows_backend() {
        let src = include_str!("windows.rs");
        let code = src.split("mod direction_tests").next().unwrap_or(src);
        let lower = code.to_lowercase();
        for forbidden in [
            "powershell.exe",
            "pwsh ",
            "pwsh\"",
            "get-vm",
            "new-vm",
            "start-vm",
            "invoke-command",
            "invoke-cimmethod",
            "cim_",
            "system.management",
            "wmic ",
        ] {
            assert!(
                !lower.contains(forbidden),
                "windows backend must not shell out: found {forbidden}"
            );
        }
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use VirtualizationReadiness as V;

    fn ready() -> ReadinessFacts {
        ReadinessFacts {
            build: Some(26100),
            x64: true,
            hypervisor_present: true,
            cpu_supports_vt: Some(true),
            firmware_vt: None,
            vmcompute_installed: Some(true),
            reboot_pending: Some(false),
            broker_installed: Some(true),
        }
    }

    #[test]
    fn a_home_pc_with_the_feature_on_is_ready() {
        assert_eq!(classify(&ready()), (V::Ready, false));
        let r = readiness_from(&ready(), Some("Core"), Some("24H2"));
        assert_eq!(r.os_name, "Windows 11 Home (24H2)");
        assert!(r.os_supported && r.broker_installed);
        assert_eq!(r.state, V::Ready);
    }

    #[test]
    fn a_fresh_pc_needs_the_feature_and_pegoles_can_turn_it_on() {
        let f = ReadinessFacts {
            hypervisor_present: false,
            firmware_vt: Some(true),
            vmcompute_installed: Some(false),
            ..ready()
        };
        assert_eq!(classify(&f), (V::NeedsEnable, true));
    }

    #[test]
    fn firmware_off_is_the_persons_step_and_restart_is_detected() {
        let off = ReadinessFacts {
            hypervisor_present: false,
            firmware_vt: Some(false),
            vmcompute_installed: Some(false),
            ..ready()
        };
        assert_eq!(classify(&off), (V::FirmwareDisabled, false));
        let pending = ReadinessFacts {
            hypervisor_present: false,
            firmware_vt: Some(true),
            vmcompute_installed: Some(true),
            reboot_pending: Some(true),
            ..ready()
        };
        assert_eq!(classify(&pending), (V::RestartPending, false));
        let just_enabled = ReadinessFacts {
            hypervisor_present: false,
            firmware_vt: Some(true),
            vmcompute_installed: Some(false),
            reboot_pending: Some(true),
            ..ready()
        };
        assert_eq!(classify(&just_enabled), (V::RestartPending, false));
    }

    #[test]
    fn unknowns_and_unsupported_are_never_ready() {
        assert_eq!(
            classify(&ReadinessFacts {
                x64: false,
                ..ready()
            })
            .0,
            V::Unsupported
        );
        assert_eq!(
            classify(&ReadinessFacts {
                vmcompute_installed: None,
                ..ready()
            })
            .0,
            V::Unknown
        );
        let no_vt = ReadinessFacts {
            hypervisor_present: false,
            cpu_supports_vt: Some(false),
            vmcompute_installed: Some(false),
            ..ready()
        };
        assert_eq!(classify(&no_vt).0, V::Unsupported);
        let no_launch = ReadinessFacts {
            hypervisor_present: false,
            firmware_vt: Some(true),
            ..ready()
        };
        assert_eq!(classify(&no_launch).0, V::Unknown);
        // Ready PC, missing broker: an install problem, not "ready".
        let r = readiness_from(
            &ReadinessFacts {
                broker_installed: Some(false),
                ..ready()
            },
            None,
            None,
        );
        assert_eq!(r.state, V::Unknown);
        assert!(r.technical.contains("Pegoles VM broker service no"));
        assert_eq!(
            readiness_from(&ReadinessFacts::default(), None, None).state,
            V::Unsupported
        );
    }

    #[test]
    fn windows_names_follow_the_build_not_the_registry_product_name() {
        assert_eq!(
            os_name(Some(22631), Some("Professional"), Some("23H2")),
            "Windows 11 Pro (23H2)"
        );
        assert_eq!(os_name(Some(19045), Some("Core"), None), "Windows 10 Home");
        assert_eq!(
            os_name(Some(26100), Some("ServerDatacenter"), None),
            "Windows Server"
        );
        assert_eq!(os_name(None, None, None), "Windows");
        let r = readiness_from(
            &ReadinessFacts {
                build: Some(19045),
                ..ready()
            },
            Some("Core"),
            None,
        );
        assert!(!r.os_supported);
    }

    #[test]
    fn service_guid_derivation_matches_microsoft_doc() {
        assert_eq!(
            hyperv_service_guid_for_port(2761),
            "00000AC9-facb-11e6-bd58-64006a7986d3"
        );
        assert_eq!(
            pegoles_hyperv_service_guid(),
            "00000352-facb-11e6-bd58-64006a7986d3"
        );
    }

    #[test]
    fn setup_steps_off_windows_are_honest_errors() {
        if cfg!(not(target_os = "windows")) {
            assert!(enable_virtualization_elevated().is_err());
            assert!(restart_for_setup().is_err());
            assert!(check_windows_platform().is_err());
        }
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::native_backend::{FakeTransport, HostTransport};
    use std::time::Duration;

    fn test_dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let images = tmp.path().join("images");
        let computers = tmp.path().join("computers");
        (tmp, images, computers)
    }

    fn seed_official_only(images_dir: &std::path::Path) {
        // Official RAW only (macOS-style source): must NEVER boot Windows.
        let dir = images_dir.join("pegoles-debian-13-arm64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base.raw"), b"fake-disk-bytes").unwrap();
        std::fs::write(dir.join("base.raw.verified"), "abc123").unwrap();
    }

    fn seed_derived_vhdx(images_dir: &std::path::Path) {
        use crate::image::{ComputerImageManager, DerivedManifestInput};
        std::fs::create_dir_all(images_dir).unwrap();
        let mgr = ComputerImageManager::new(images_dir.to_path_buf());
        let work = images_dir.join("work.vhdx");
        std::fs::write(&work, b"fake-vhdx-bytes").unwrap();
        mgr.publish_derived_as(
            &work,
            DerivedManifestInput::v0_1(
                "13".into(),
                "amd64".into(),
                "0.1.0".into(),
                1,
                "sourcesha".into(),
            )
            .for_active_image(),
            "disk.vhdx",
            DiskFormat::Vhdx,
        )
        .unwrap();
    }

    fn backend(
        images: std::path::PathBuf,
        computers: std::path::PathBuf,
    ) -> (
        WindowsHcsBackend,
        std::sync::Arc<std::sync::Mutex<FakeTransport>>,
    ) {
        let fake = std::sync::Arc::new(std::sync::Mutex::new(FakeTransport::new()));
        struct Shared(std::sync::Arc<std::sync::Mutex<FakeTransport>>);
        impl HostTransport for Shared {
            fn send(&mut self, line: &str) -> Result<()> {
                self.0.lock().expect("fake").send(line)
            }
            fn recv(&mut self, timeout: Duration) -> Result<String> {
                self.0.lock().expect("fake").recv(timeout)
            }
            fn try_recv(&mut self) -> Option<Result<String>> {
                self.0.lock().expect("fake").try_recv()
            }
            fn alive(&mut self) -> bool {
                self.0.lock().expect("fake").alive()
            }
        }
        let b =
            WindowsHcsBackend::with_transport(images, computers, Box::new(Shared(fake.clone())));
        (b, fake)
    }

    #[test]
    fn create_uses_vhdx_layout_with_serial_log() {
        let (_tmp, images, computers) = test_dirs();
        seed_derived_vhdx(&images);
        let (mut b, _fake) = backend(images, computers.clone());
        let id = b.create(crate::config::default_config()).unwrap();
        let dir = computers.join(id.to_string());
        assert!(
            dir.join("disk.vhdx").is_file(),
            "windows disk must be .vhdx"
        );
        assert!(
            !dir.join("disk.img").exists(),
            "no RAW disk on the Windows path"
        );
        assert!(dir.join("metadata.json").is_file());
        assert!(
            dir.join("logs").is_dir(),
            "the helper writes COM1 into logs/serial.log"
        );
    }

    #[test]
    fn official_raw_never_boots_windows() {
        // Derived VHDX missing + official RAW present: create must fail
        // closed as ImageMissing, never silently boot the wrong disk.
        let (_tmp, images, computers) = test_dirs();
        seed_official_only(&images);
        let (mut b, _fake) = backend(images, computers);
        let err = b.create(crate::config::default_config()).unwrap_err();
        assert!(
            matches!(err, ComputerError::ImageMissing(_)),
            "expected ImageMissing, got {err:?}"
        );
    }

    #[test]
    fn lifecycle_and_instance_split() {
        use crate::traits::ComputerBackend;
        let (_tmp, images, computers) = test_dirs();
        seed_derived_vhdx(&images);
        let (mut b, _fake) = backend(images, computers);
        let computer = b.create(crate::config::default_config()).unwrap();
        assert!(b.instance().is_none());
        assert_eq!(b.start().unwrap(), ComputerState::Running);
        let first = b.instance().expect("instance while running");
        assert_eq!(first.computer_id, computer);
        assert_eq!(b.pause().unwrap(), ComputerState::Paused);
        assert_eq!(b.resume().unwrap(), ComputerState::Running);
        assert_eq!(b.stop().unwrap(), ComputerState::Stopped);
        assert!(b.instance().is_none());
        // Second start: same computer, NEW ephemeral instance.
        assert_eq!(b.start().unwrap(), ComputerState::Running);
        let second = b.instance().expect("instance while running");
        assert_eq!(second.computer_id, computer);
        assert_ne!(first.instance_id, second.instance_id);
        assert_eq!(b.stop().unwrap(), ComputerState::Stopped);
    }

    #[test]
    fn wrapper_capabilities_match_hcs_reality() {
        use crate::traits::ComputerBackend;
        let (_tmp, images, computers) = test_dirs();
        seed_derived_vhdx(&images);
        let (b, _fake) = backend(images, computers);
        let caps = b.capabilities();
        assert!(caps.pause && caps.resume && caps.vsock);
        assert!(!caps.snapshot && !caps.graphical_display);
        assert_eq!(caps.guest_arch, GuestArchitecture::X86_64);
        assert_eq!(caps.disk_formats, vec![DiskFormat::Vhdx]);
    }

    #[test]
    fn guest_handshake_to_ready_via_fake() {
        use crate::guest::GuestObservation;
        use crate::traits::ComputerBackend;
        use pegoles_protocol::GuestRuntimeState as G;
        let (_tmp, images, computers) = test_dirs();
        seed_derived_vhdx(&images);
        let (mut b, fake) = backend(images, computers);
        let id = b.create(crate::config::default_config()).unwrap();
        b.start().unwrap();
        assert_eq!(b.guest_state(), G::Waiting);
        let cid = id.to_string();
        {
            let mut f = fake.lock().expect("fake");
            f.inject_guest_connected(&cid);
            f.inject_guest_frame(
                &cid,
                r#"{"type":"guest_hello","protocol_version":1,"runtime_version":"0.1.0","os":"debian","os_version":"13","arch":"x86_64"}"#,
            );
            f.inject_guest_frame(&cid, r#"{"type":"ready"}"#);
        }
        let obs = b.poll_guest();
        assert!(obs
            .iter()
            .any(|o| matches!(o, GuestObservation::BecameReady { .. })));
        assert_eq!(b.guest_state(), G::Ready);
    }
}

#[cfg(test)]
mod e2e_tests {
    use super::*;
    use crate::traits::ComputerBackend;
    use pegoles_protocol::{ComputerState, GuestRuntimeState as G};

    /// Real Windows E2E through the product path (helper + broker + HCS +
    /// HvSocket + the amd64 image). Skipped unless
    /// PEGOLES_REAL_WINDOWS_VM_TEST=1; needs virtualization ready, the
    /// broker installed and the Windows image installed.
    #[test]
    fn real_windows_e2e_smoke() {
        if std::env::var("PEGOLES_REAL_WINDOWS_VM_TEST").as_deref() != Ok("1") {
            eprintln!("skipping real Windows e2e (set PEGOLES_REAL_WINDOWS_VM_TEST=1)");
            return;
        }
        let r = readiness();
        assert_eq!(
            r.state,
            VirtualizationReadiness::Ready,
            "host not ready: {}",
            r.technical
        );
        assert!(r.broker_installed, "install the broker service first");
        let data = crate::config::pegoles_data_dir();
        let mut backend = WindowsHcsBackend::new(data.join("images"), data.join("computers"))
            .expect("backend constructor");
        let id = backend
            .create(crate::config::default_config())
            .expect("create");
        let t = std::time::Instant::now();
        assert_eq!(backend.start().expect("start"), ComputerState::Running);
        eprintln!("vm start -> Running: {:?}", t.elapsed());
        let first = backend.instance().expect("instance");
        let wait_ready = |backend: &mut WindowsHcsBackend, what: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
            while backend.guest_state() != G::Ready {
                let _ = backend.poll_guest();
                assert!(
                    !matches!(backend.guest_state(), G::Incompatible | G::Error),
                    "{what}: {:?}",
                    backend.guest_state()
                );
                assert!(std::time::Instant::now() < deadline, "{what}: never Ready");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        };
        wait_ready(&mut backend, "first start");
        eprintln!("guest -> Ready in {:?} ms", backend.guest_ready_ms());
        let info = backend
            .guest_info_request(std::time::Duration::from_secs(15))
            .expect("system info");
        assert_eq!(info.os, "debian");
        assert!(info.arch == "x86_64" || info.arch == "amd64");
        assert_eq!(backend.pause().expect("pause"), ComputerState::Paused);
        assert_eq!(backend.resume().expect("resume"), ComputerState::Running);
        wait_ready(&mut backend, "resume");
        assert_eq!(backend.stop().expect("stop"), ComputerState::Stopped);
        assert_eq!(
            backend.start().expect("second start"),
            ComputerState::Running
        );
        let second = backend.instance().expect("instance");
        assert_eq!(second.computer_id, id);
        assert_ne!(first.instance_id, second.instance_id);
        wait_ready(&mut backend, "second start");
        assert_eq!(backend.stop().expect("final stop"), ComputerState::Stopped);
        backend.destroy().expect("destroy");
    }
}

//! Windows HCS backend skeleton (Phase 3.5: architecture, not a VM).
//!
//! Future: `WindowsHcsBackend` drives Hyper-V through Host Compute System
//! APIs via `native/windows/pegoles-vm-host`, with the guest control plane
//! over Hyper-V sockets (AF_HYPERV host / AF_VSOCK Linux guest, same Guest
//! Protocol v1). See `docs/WINDOWS_BACKEND.md`.
//!
//! In this phase the backend compiles everywhere, detects its platform
//! honestly, classifies host support without inventing results, and every
//! unimplemented lifecycle operation returns `BackendFeatureNotImplemented`
//! explicitly. No Mock fallback, no PowerShell, no GUI automation.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, SnapshotId};
use serde::{Deserialize, Serialize};

use crate::error::{ComputerError, Result};
use crate::platform::{BackendCapabilities, DiskFormat, GuestArchitecture, HostArchitecture};
use crate::traits::ComputerBackend;

/// Registry path where the Pegoles Hyper-V socket service must be
/// registered (one-time privileged installer step; runtime only verifies).
/// Full key: HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\
///   Virtualization\GuestCommunicationServices\<service-guid>
pub const HV_SOCKET_REGISTRY_PATH: &str =
    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices";

/// Well-known VSOCK template GUID from Microsoft's "Make your own
/// integration services" doc: Data1 carries the Linux guest port, e.g.
/// `00000AC9-facb-…` is port 2761.
pub const HV_VSOCK_TEMPLATE_SUFFIX: &str = "facb-11e6-bd58-64006a7986d3";

/// Deterministic Hyper-V service GUID for a Linux-guest VSOCK port.
/// Single source of truth — never hardcode GUIDs elsewhere.
pub fn hyperv_service_guid_for_port(port: u32) -> String {
    format!("{port:08X}-{HV_VSOCK_TEMPLATE_SUFFIX}")
}

/// The Pegoles guest control-plane endpoint on Hyper-V.
pub fn pegoles_hyperv_service_guid() -> String {
    hyperv_service_guid_for_port(pegoles_guest_proto::PEGOLES_VSOCK_PORT)
}

/// User-facing Windows support states. Never collapse these into a bare
/// "Windows unsupported": each state tells the user exactly what is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSupport {
    Supported,
    SetupRequired,
    UnsupportedEdition,
    VirtualizationDisabled,
    HyperVDisabled,
    RebootRequired,
    PermissionMissing,
    HcsUnavailable,
    HvSocketRegistrationMissing,
    UnsupportedArchitecture,
}

/// Windows host facts. Every detection field is `Option`: unknown is
/// unknown, never assumed. `state` is always set by `classify()`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsHostCapabilities {
    pub windows_version: Option<String>,
    pub edition: Option<String>,
    pub architecture: HostArchitecture,
    pub hardware_virtualization: Option<bool>,
    pub hyperv_available: Option<bool>,
    pub hyperv_enabled: Option<bool>,
    pub hcs_available: Option<bool>,
    pub hv_socket_registered: Option<bool>,
    pub permission_ok: Option<bool>,
    pub reboot_required: Option<bool>,
    pub state: WindowsSupport,
    pub notes: Vec<String>,
}

impl WindowsHostCapabilities {
    /// Ordered, human-readable setup steps for the current state.
    /// Empty only when Supported.
    pub fn setup_steps(&self) -> Vec<String> {
        match self.state {
            WindowsSupport::Supported => Vec::new(),
            WindowsSupport::UnsupportedEdition => vec![
                "Windows 11 Pro (or Enterprise/Education) is required; Home is not supported by the HCS backend (a future WHP backend is research-only)".to_string(),
            ],
            WindowsSupport::UnsupportedArchitecture => vec![
                "WindowsHcsBackend targets x86_64 first; this architecture is not supported yet".to_string(),
            ],
            WindowsSupport::VirtualizationDisabled => vec![
                "Enable hardware virtualization (Intel VT-x / AMD-V) in firmware settings".to_string(),
            ],
            WindowsSupport::HyperVDisabled => vec![
                "Enable Hyper-V in 'Turn Windows features on or off', then reboot".to_string(),
            ],
            WindowsSupport::RebootRequired => {
                vec!["Reboot Windows to finish enabling Hyper-V".to_string()]
            }
            WindowsSupport::PermissionMissing => vec![
                "Run setup once with Hyper-V Administrators membership, then use Pegoles unprivileged".to_string(),
            ],
            WindowsSupport::HcsUnavailable => vec![
                "Host Compute System APIs are unavailable on this machine".to_string(),
            ],
            WindowsSupport::HvSocketRegistrationMissing => vec![
                format!("Run the one-time privileged installer to register the Pegoles socket service ({HV_SOCKET_REGISTRY_PATH}\\<service-guid>)"),
            ],
            WindowsSupport::SetupRequired => self
                .notes
                .iter()
                .map(|n| format!("Setup required: {n}"))
                .collect(),
        }
    }
}

/// Pure support classifier: all evidence in, one state out. Fully unit
/// tested on every OS. Order matters: edition/arch first (hard blockers),
/// then firmware, feature, reboot, permissions, HCS, socket registration.
pub fn classify_windows_support(c: &WindowsHostCapabilities) -> WindowsSupport {
    if matches!(c.architecture, HostArchitecture::Arm64) {
        return WindowsSupport::UnsupportedArchitecture;
    }
    if let Some(edition) = c.edition.as_deref() {
        let edition = edition.to_lowercase();
        if edition.contains("home") {
            return WindowsSupport::UnsupportedEdition;
        }
    }
    if c.hardware_virtualization == Some(false) {
        return WindowsSupport::VirtualizationDisabled;
    }
    if c.hyperv_available == Some(false) || c.hyperv_enabled == Some(false) {
        return WindowsSupport::HyperVDisabled;
    }
    if c.reboot_required == Some(true) {
        return WindowsSupport::RebootRequired;
    }
    if c.permission_ok == Some(false) {
        return WindowsSupport::PermissionMissing;
    }
    if c.hcs_available == Some(false) {
        return WindowsSupport::HcsUnavailable;
    }
    if c.hv_socket_registered == Some(false) {
        return WindowsSupport::HvSocketRegistrationMissing;
    }
    // All green (explicitly true everywhere we checked).
    let proven = c.hardware_virtualization == Some(true)
        && c.hyperv_available == Some(true)
        && c.hyperv_enabled == Some(true)
        && c.hcs_available == Some(true)
        && c.hv_socket_registered == Some(true)
        && c.permission_ok != Some(false);
    if proven {
        return WindowsSupport::Supported;
    }
    WindowsSupport::SetupRequired
}

/// Probe this host. Off Windows this is honestly inapplicable
/// (`UnsupportedPlatform`, never invented data). On Windows every item is
/// detected with a documented OS API; anything a call cannot prove stays
/// `None` (Unknown), and `classify()` turns unknowns into SetupRequired.
pub fn probe_windows_host() -> Result<WindowsHostCapabilities> {
    #[cfg(not(target_os = "windows"))]
    {
        Err(ComputerError::UnsupportedPlatform(
            "Windows host probe runs on Windows only".to_string(),
        ))
    }
    #[cfg(target_os = "windows")]
    {
        self::detect::probe()
    }
}

/// Live Windows detection, isolated so every call is reviewable and no
/// failure poisons the whole probe (one bad call => that field Unknown).
#[cfg(target_os = "windows")]
mod detect {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Security::{
        AllocateAndInitializeSid, CheckTokenMembership, FreeSid, DOMAIN_ALIAS_RID_ADMINS,
        SECURITY_NT_AUTHORITY,
    };
    use windows::Win32::System::LibraryLoader::{FreeLibrary, LoadLibraryW};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
    };
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_MANAGER_CONNECT,
        SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS_PROCESS,
    };
    use windows::Win32::System::SystemInformation::{
        IsProcessorFeaturePresent, PF_VIRT_FIRMWARE_ENABLED,
    };

    /// BUILTIN\Hyper-V Administrators, S-1-5-32-578 (Microsoft well-known
    /// SID, same family as DOMAIN_ALIAS_RID_ADMINS = 544).
    const DOMAIN_ALIAS_RID_HYPER_V_ADMINS: u32 = 578;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Read a REG_SZ string value. None = missing/unreadable (Unknown).
    fn reg_string(hive: HKEY, subkey: &str, value: &str) -> Option<String> {
        unsafe {
            let mut key = HKEY::default();
            let subkey = wide(subkey);
            if RegOpenKeyExW(hive, PCWSTR(subkey.as_ptr()), None, KEY_READ, &mut key).is_err() {
                return None;
            }
            let mut kind = REG_SZ;
            let mut size: u32 = 0;
            let vname = wide(value);
            // Size query first, then the read.
            if RegQueryValueExW(
                key,
                PCWSTR(vname.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
            .is_err()
            {
                let _ = RegCloseKey(key);
                return None;
            }
            let mut buf = vec![0u16; (size as usize / 2).max(1)];
            let ok = RegQueryValueExW(
                key,
                PCWSTR(vname.as_ptr()),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            )
            .is_ok();
            let _ = RegCloseKey(key);
            if !ok {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16(&buf[..len]).ok()
        }
    }

    fn reg_key_exists(hive: HKEY, subkey: &str) -> Option<bool> {
        unsafe {
            let mut key = HKEY::default();
            let subkey = wide(subkey);
            let opened =
                RegOpenKeyExW(hive, PCWSTR(subkey.as_ptr()), None, KEY_READ, &mut key).is_ok();
            if opened {
                let _ = RegCloseKey(key);
                Some(true)
            } else {
                // Absent key vs access denied are both "not proven present".
                // Access problems surface separately via permission checks.
                Some(false)
            }
        }
    }

    const NT_CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    fn windows_version() -> Option<String> {
        let display = reg_string(HKEY_LOCAL_MACHINE, NT_CURRENT_VERSION, "DisplayVersion");
        let build = reg_string(HKEY_LOCAL_MACHINE, NT_CURRENT_VERSION, "CurrentBuildNumber");
        match (display, build) {
            (Some(d), Some(b)) => Some(format!("Windows {d} (build {b})")),
            (Some(d), None) => Some(format!("Windows {d}")),
            (None, Some(b)) => Some(format!("Windows (build {b})")),
            (None, None) => None,
        }
    }

    fn edition() -> Option<String> {
        reg_string(HKEY_LOCAL_MACHINE, NT_CURRENT_VERSION, "EditionID")
    }

    /// Firmware-enabled virtualization (Intel VT-x / AMD-V) via the
    /// documented processor feature flag. No WMI, no shell.
    fn hardware_virtualization() -> Option<bool> {
        Some(unsafe { IsProcessorFeaturePresent(PF_VIRT_FIRMWARE_ENABLED).as_bool() })
    }

    /// Hyper-V availability/enabled via the vmms service: exists =>
    /// available; running => enabled. Real SCM API, no shell-outs.
    fn hyperv_service() -> (Option<bool>, Option<bool>) {
        unsafe {
            let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT);
            let Ok(scm) = scm else { return (None, None) };
            let svc = OpenServiceW(scm, windows::core::w!("vmms"), SERVICE_QUERY_STATUS);
            let _ = CloseServiceHandle(scm);
            let Ok(svc) = svc else {
                return (Some(false), Some(false));
            };
            let mut status = SERVICE_STATUS_PROCESS::default();
            let mut needed = 0u32;
            let ok = QueryServiceStatusEx(
                svc,
                SC_STATUS_PROCESS_INFO,
                Some(std::slice::from_raw_parts_mut(
                    &mut status as *mut _ as *mut u8,
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
                )),
                &mut needed,
                None,
            )
            .is_ok();
            let _ = CloseServiceHandle(svc);
            if !ok {
                return (Some(true), None);
            }
            use windows::Win32::System::Services::SERVICE_RUNNING;
            (Some(true), Some(status.dwCurrentState == SERVICE_RUNNING))
        }
    }

    /// HCS presence = ComputeCore.dll loadable. No calls made, no side
    /// effects; the handle is freed immediately.
    fn hcs_available() -> Option<bool> {
        unsafe {
            match LoadLibraryW(&wide("computecore.dll")) {
                Ok(handle) => {
                    let _ = FreeLibrary(handle);
                    Some(true)
                }
                Err(_) => Some(false),
            }
        }
    }

    /// Pegoles Hyper-V socket service registration: our GUID subkey must
    /// exist under GuestCommunicationServices. Verify-only (runtime never
    /// writes; the installer does).
    fn hv_socket_registered() -> Option<bool> {
        let subkey = format!(
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices\{}",
            super::pegoles_hyperv_service_guid()
        );
        reg_key_exists(HKEY_LOCAL_MACHINE, &subkey)
    }

    /// Current token membership in a BUILTIN alias group (RID). Used for
    /// Administrators (544) and Hyper-V Administrators (578). Unknown on
    /// any API failure — never guessed.
    fn is_member_of(rid: u32) -> Option<bool> {
        unsafe {
            let mut sid: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut auth = SECURITY_NT_AUTHORITY;
            AllocateAndInitializeSid(&mut auth, 1, rid, 0, 0, 0, 0, 0, 0, 0, &mut sid).ok()?;
            if sid.is_null() {
                return None;
            }
            let mut member = BOOL::default();
            let res = CheckTokenMembership(None, sid, &mut member)
                .ok()
                .map(|_| member.as_bool());
            FreeSid(sid);
            res
        }
    }

    /// Pending-reboot heuristic (standard keys; documented as heuristic):
    /// PendingFileRenameOperations non-empty or CBS RebootPending present.
    fn reboot_required() -> Option<bool> {
        unsafe {
            // PendingFileRenameOperations is REG_MULTI_SZ; presence check
            // via a size query is enough for the heuristic.
            let mut key = HKEY::default();
            let sub = wide(r"SYSTEM\CurrentControlSet\Control\Session Manager");
            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_READ,
                &mut key,
            )
            .is_err()
            {
                return None;
            }
            let vname = wide("PendingFileRenameOperations");
            let mut kind = REG_SZ;
            let mut size: u32 = 0;
            let pending = RegQueryValueExW(
                key,
                PCWSTR(vname.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
            .is_ok()
                && size > 0;
            let _ = RegCloseKey(key);
            if pending {
                return Some(true);
            }
            // Component Based Servicing marker.
            let cbs = wide(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
            );
            let mut cbs_key = HKEY::default();
            let pending_cbs = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(cbs.as_ptr()),
                None,
                KEY_READ,
                &mut cbs_key,
            )
            .is_ok();
            if pending_cbs {
                let _ = RegCloseKey(cbs_key);
                return Some(true);
            }
            Some(false)
        }
    }

    pub(super) fn probe() -> Result<WindowsHostCapabilities> {
        let (hyperv_available, hyperv_enabled) = hyperv_service();
        // Either Administrators or Hyper-V Administrators suffices to
        // drive Hyper-V; require at least one proven.
        let admin = is_member_of(DOMAIN_ALIAS_RID_ADMINS);
        let hv_admin = is_member_of(DOMAIN_ALIAS_RID_HYPER_V_ADMINS);
        let permission_ok = match (admin, hv_admin) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
        let mut caps = WindowsHostCapabilities {
            windows_version: windows_version(),
            edition: edition(),
            architecture: crate::platform::host_architecture(),
            hardware_virtualization: hardware_virtualization(),
            hyperv_available,
            hyperv_enabled,
            hcs_available: hcs_available(),
            hv_socket_registered: hv_socket_registered(),
            permission_ok,
            reboot_required: reboot_required(),
            state: WindowsSupport::SetupRequired,
            notes: vec![
                "probed with OS APIs (registry, SCM, security); unproven items stay Unknown"
                    .to_string(),
            ],
        };
        caps.state = classify_windows_support(&caps);
        Ok(caps)
    }

    // Silence unused-import lints for items kept for documented next steps.
    #[allow(dead_code)]
    fn _linked() {}
}

/// Real Windows backend (Phase 3.6): persistent `ComputerId` identity with
/// ephemeral HCS compute-system handles per start (see `ComputerInstance`).
/// Thin wrapper like its macOS sibling: profile + platform gate here, all
/// behavior in the shared [`NativeHelperBackend`] engine, driven through
/// `native/windows/pegoles-vm-host` over the same JSONL command set.
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
            "../../native/windows/pegoles-vm-host/target/release/pegoles-vm-host.exe",
            "../../native/windows/pegoles-vm-host/target/debug/pegoles-vm-host.exe",
            "../../native/windows/pegoles-vm-host/target/release/pegoles-vm-host",
            "../../native/windows/pegoles-vm-host/target/debug/pegoles-vm-host",
        ],
        resource_helper_name: "pegoles-vm-host.exe",
        // No official RAW fallback on Windows: only the derived VHDX boots
        // on Hyper-V. Missing derived image fails closed as ImageMissing.
        official_spec: crate::image::GENERIC_DEBIAN_13_ARM64,
        allow_official_fallback: false,
        disk_file_name: "disk.vhdx",
        disk_format: DiskFormat::Vhdx,
        want_serial_log: false,
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

/// Platform gate: Windows only, x86_64 first, Home refused with its
/// future path. Called by `new()`; never a silent Mock fallback.
pub fn check_windows_platform() -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        Err(ComputerError::UnsupportedPlatform(
            "WindowsHcsBackend runs on Windows only".to_string(),
        ))
    }
    #[cfg(target_os = "windows")]
    {
        if !cfg!(target_arch = "x86_64") {
            return Err(ComputerError::UnsupportedArchitecture(
                "WindowsHcsBackend targets x86_64 first".to_string(),
            ));
        }
        match probe_windows_host() {
            Ok(caps) => match classify_windows_support(&caps) {
                WindowsSupport::Supported => Ok(()),
                WindowsSupport::SetupRequired => {
                    Err(ComputerError::SetupRequired(caps.setup_steps().join("; ")))
                }
                other => Err(ComputerError::UnsupportedPlatform(format!(
                    "windows host not ready: {other:?} ({})",
                    caps.setup_steps().join("; ")
                ))),
            },
            Err(e) => Err(e),
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
    fn input_capture_frame(
        &mut self,
        request_id: &str,
        timeout: std::time::Duration,
    ) -> Result<crate::input::CapturedFrame> {
        self.engine.input_capture_frame(request_id, timeout)
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
mod tests {
    use super::*;

    fn healthy() -> WindowsHostCapabilities {
        WindowsHostCapabilities {
            windows_version: Some("11".into()),
            edition: Some("Pro".into()),
            architecture: HostArchitecture::X86_64,
            hardware_virtualization: Some(true),
            hyperv_available: Some(true),
            hyperv_enabled: Some(true),
            hcs_available: Some(true),
            hv_socket_registered: Some(true),
            permission_ok: Some(true),
            reboot_required: Some(false),
            state: WindowsSupport::SetupRequired,
            notes: Vec::new(),
        }
    }

    fn with(
        mut base: WindowsHostCapabilities,
        f: impl FnOnce(&mut WindowsHostCapabilities),
    ) -> WindowsHostCapabilities {
        f(&mut base);
        base
    }

    #[test]
    fn all_green_is_supported() {
        assert_eq!(
            classify_windows_support(&healthy()),
            WindowsSupport::Supported
        );
        assert!(
            healthy().setup_steps().is_empty() || {
                let mut c = healthy();
                c.state = WindowsSupport::Supported;
                c.setup_steps().is_empty()
            }
        );
    }

    #[test]
    fn home_edition_blocked_with_future_path() {
        let c = with(healthy(), |c| c.edition = Some("Home".into()));
        assert_eq!(
            classify_windows_support(&c),
            WindowsSupport::UnsupportedEdition
        );
        let mut c = c;
        c.state = WindowsSupport::UnsupportedEdition;
        let steps = c.setup_steps().join(" ");
        assert!(steps.contains("Home") && steps.contains("WHP"));
    }

    #[test]
    fn arm64_windows_is_explicit_not_silent() {
        let c = with(healthy(), |c| c.architecture = HostArchitecture::Arm64);
        assert_eq!(
            classify_windows_support(&c),
            WindowsSupport::UnsupportedArchitecture
        );
    }

    #[test]
    fn each_missing_piece_maps_to_its_state() {
        let base = with(healthy(), |c| c.hardware_virtualization = Some(false));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::VirtualizationDisabled
        );
        let base = with(healthy(), |c| c.hyperv_enabled = Some(false));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::HyperVDisabled
        );
        let base = with(healthy(), |c| c.reboot_required = Some(true));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::RebootRequired
        );
        let base = with(healthy(), |c| c.permission_ok = Some(false));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::PermissionMissing
        );
        let base = with(healthy(), |c| c.hcs_available = Some(false));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::HcsUnavailable
        );
        let base = with(healthy(), |c| c.hv_socket_registered = Some(false));
        assert_eq!(
            classify_windows_support(&base),
            WindowsSupport::HvSocketRegistrationMissing
        );
    }

    #[test]
    fn unknown_means_setup_required_never_supported() {
        let c = with(healthy(), |c| {
            c.edition = None;
            c.hardware_virtualization = None;
            c.hyperv_available = None;
            c.hyperv_enabled = None;
            c.hcs_available = None;
            c.hv_socket_registered = None;
            c.permission_ok = None;
            c.reboot_required = None;
        });
        assert_eq!(classify_windows_support(&c), WindowsSupport::SetupRequired);
    }

    #[test]
    fn service_guid_derivation_matches_microsoft_doc() {
        // Doc example: 0x00000AC9 == port 2761.
        assert_eq!(
            hyperv_service_guid_for_port(2761),
            "00000AC9-facb-11e6-bd58-64006a7986d3"
        );
        // Pegoles control-plane port.
        assert_eq!(
            hyperv_service_guid_for_port(4050),
            "00000FD2-facb-11e6-bd58-64006a7986d3"
        );
        assert_eq!(
            pegoles_hyperv_service_guid(),
            hyperv_service_guid_for_port(pegoles_guest_proto::PEGOLES_VSOCK_PORT)
        );
    }

    #[test]
    fn probe_off_windows_is_honest() {
        #[cfg(not(target_os = "windows"))]
        assert!(matches!(
            probe_windows_host(),
            Err(ComputerError::UnsupportedPlatform(_))
        ));
    }

    #[test]
    fn skeleton_never_pretends_to_work() {
        #[cfg(not(target_os = "windows"))]
        {
            let tmp = tempfile::tempdir().unwrap();
            assert!(matches!(
                WindowsHcsBackend::new(tmp.path().join("i"), tmp.path().join("c")),
                Err(ComputerError::UnsupportedPlatform(_))
            ));
        }
    }

    #[test]
    fn windows_capabilities_describe_hcs_reality() {
        // HCS path: pause/resume/vsock are real operations (driven through
        // the native helper); snapshots and dynamic memory are not.
        let caps = BackendCapabilities {
            pause: true,
            resume: true,
            snapshot: false,
            graphical_display: false,
            vsock: true,
            dynamic_memory: false,
            guest_arch: GuestArchitecture::X86_64,
            disk_formats: vec![DiskFormat::Vhdx],
        };
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(v["guest_arch"], "x86_64");
        assert_eq!(v["disk_formats"], serde_json::json!(["vhdx"]));
        assert_eq!(v["vsock"], true);
        assert_eq!(v["snapshot"], false);
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
            ),
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
    fn create_uses_vhdx_layout_without_serial_log() {
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
            !dir.join("logs").join("serial.log").exists(),
            "no virtio serial capture on Windows (HCS COM capture is future work)"
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

    /// Real Windows hardware E2E. Skipped unless
    /// PEGOLES_REAL_WINDOWS_VM_TEST=1. Requires: Windows 11 Pro x86_64,
    /// Hyper-V enabled, socket service registered, derived VHDX Ready.
    /// Never runs in CI. Flow covers the critical second start proving
    /// persistent-Computer / ephemeral-instance semantics.
    #[test]
    fn real_windows_e2e_smoke() {
        if std::env::var("PEGOLES_REAL_WINDOWS_VM_TEST").as_deref() != Ok("1") {
            eprintln!("skipping real Windows e2e (set PEGOLES_REAL_WINDOWS_VM_TEST=1)");
            return;
        }
        assert_eq!(
            crate::platform::host_platform(),
            crate::platform::HostPlatform::Windows,
            "Windows hardware required"
        );
        // 1. Host capability validation (explicit, never assumed).
        let caps = probe_windows_host().expect("windows probe");
        assert_eq!(
            caps.state,
            WindowsSupport::Supported,
            "host not ready: {:?} ({:?})",
            caps.state,
            caps.setup_steps()
        );
        // 2. Socket registration validation (setup path, not a late failure).
        assert_eq!(
            caps.hv_socket_registered,
            Some(true),
            "register the Pegoles socket service first (see WINDOWS_SETUP.md)"
        );
        // 3. Derived VHDX present.
        let data = crate::config::pegoles_data_dir();
        let mgr = crate::image::ComputerImageManager::new(data.join("images"));
        assert_eq!(
            mgr.derived_status(),
            crate::image::ImageStatus::Ready,
            "build the Windows VHDX artifact first"
        );

        let mut backend = WindowsHcsBackend::new(data.join("images"), data.join("computers"))
            .expect("backend constructor");
        let t0 = std::time::Instant::now();
        let id = backend
            .create(crate::config::default_config())
            .expect("create");
        eprintln!("create took {:?}", t0.elapsed());

        // 4. Start -> Running.
        let t1 = std::time::Instant::now();
        assert_eq!(backend.start().expect("start"), ComputerState::Running);
        eprintln!("vm start -> Running: {:?}", t1.elapsed());
        let first_instance = backend.instance().expect("instance after start");
        assert_eq!(first_instance.computer_id, id);

        // 5. Guest handshake -> Ready.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            let _ = backend.poll_guest();
            if backend.guest_state() == G::Ready {
                break;
            }
            if matches!(backend.guest_state(), G::Incompatible | G::Error) {
                panic!("guest failed: {:?}", backend.guest_state());
            }
            if std::time::Instant::now() >= deadline {
                panic!("guest never Ready: {:?}", backend.guest_state());
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let ready_ms = backend.guest_ready_ms().expect("ready duration");
        eprintln!("guest -> Ready in {ready_ms} ms");

        // 6. Ping/Pong + SystemInfo assertions.
        let ping = backend
            .guest_ping(std::time::Duration::from_secs(15))
            .expect("ping");
        eprintln!("ping -> pong: {ping} ms");
        let info = backend
            .guest_info_request(std::time::Duration::from_secs(15))
            .expect("system info");
        eprintln!(
            "system info: {} {} kernel={} arch={} runtime={} proto={}",
            info.os,
            info.os_version,
            info.kernel,
            info.arch,
            info.runtime_version,
            info.protocol_version
        );
        assert_eq!(info.os, "debian");
        assert_eq!(info.os_version, "13");
        assert!(info.arch == "x86_64" || info.arch == "amd64");
        assert_eq!(info.protocol_version, 1);

        // 7. Pause/resume with guest recovery.
        assert_eq!(backend.pause().expect("pause"), ComputerState::Paused);
        assert_eq!(backend.resume().expect("resume"), ComputerState::Running);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let _ = backend.poll_guest();
            if backend.guest_state() == G::Ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("guest never re-Ready after resume");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        eprintln!("resume -> GuestReady: recovered");

        // 8. Stop, then SECOND start: same ComputerId, NEW instance.
        assert_eq!(backend.stop().expect("stop"), ComputerState::Stopped);
        assert!(backend.instance().is_none());
        assert_eq!(
            backend.start().expect("second start"),
            ComputerState::Running
        );
        let second_instance = backend.instance().expect("instance after restart");
        assert_eq!(second_instance.computer_id, id, "same Pegoles computer");
        assert_ne!(
            first_instance.instance_id, second_instance.instance_id,
            "new ephemeral HCS instance required"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            let _ = backend.poll_guest();
            if backend.guest_state() == G::Ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("guest never Ready after second start");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        eprintln!("second start -> Ready: persistent identity confirmed");

        // 9. Destroy releases everything.
        assert_eq!(backend.stop().expect("final stop"), ComputerState::Stopped);
        backend.destroy().expect("destroy");
        eprintln!("destroy: ok (computer {id} fully released)");
    }
}

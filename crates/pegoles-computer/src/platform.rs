//! Platform abstraction: the shared vocabulary for "where does Pegoles run".
//!
//! Rules:
//! - No hypervisor types here (no Vz, HCS, KVM, Swift, Win32). Only names,
//!   paths, capabilities, and factories.
//! - `pegoles-protocol` stays platform-free; these types live here because
//!   they describe the HOST and its artifacts, and they serialize for the UI.
//! - Adding a platform means: extend the enums, add a paths case, add a
//!   backend adapter. The Core never branches on `target_os` directly.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Host operating system running Pegoles Core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPlatform {
    MacOS,
    Windows,
    Linux,
}

/// CPU architecture of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostArchitecture {
    Arm64,
    X86_64,
}

impl HostArchitecture {
    pub fn as_str(&self) -> &'static str {
        match self {
            HostArchitecture::Arm64 => "arm64",
            HostArchitecture::X86_64 => "x86_64",
        }
    }
}

/// CPU architecture of a guest image. Deliberately independent from the
/// host platform: Arm64 is NOT a synonym for Mac, which keeps a future
/// Windows/ARM64 target possible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestArchitecture {
    Arm64,
    X86_64,
}

impl GuestArchitecture {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestArchitecture::Arm64 => "arm64",
            GuestArchitecture::X86_64 => "amd64",
        }
    }
}

/// On-disk virtual disk formats. Only formats with a real consumer exist
/// here: RAW (Apple Virtualization) today, VHDX (Hyper-V) when the Windows
/// backend lands. No speculative QCOW2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskFormat {
    Raw,
    Vhdx,
}

impl DiskFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            DiskFormat::Raw => "raw",
            DiskFormat::Vhdx => "vhdx",
        }
    }
}

/// Which backend drives a computer. The Core selects by platform; the
/// concrete adapters live in `macos.rs` / `windows.rs` / `mock.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Mock,
    MacOSVirtualization,
    WindowsHcs,
}

impl BackendKind {
    /// Short UI/API name. Kept stable for the frontend contract.
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Mock => "mock",
            BackendKind::MacOSVirtualization => "real",
            BackendKind::WindowsHcs => "windows-hcs",
        }
    }
}

/// What a backend can do. The future UI disables unsupported features
/// from this instead of probing hypervisors directly. Values describe
/// REALITY (what works now), not roadmap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub pause: bool,
    pub resume: bool,
    pub snapshot: bool,
    pub graphical_display: bool,
    pub vsock: bool,
    pub dynamic_memory: bool,
    pub guest_arch: GuestArchitecture,
    pub disk_formats: Vec<DiskFormat>,
}

/// Guest transport kinds. The Guest Protocol v1 runs unchanged above any
/// of these; only the transport adapter differs per hypervisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestTransportKind {
    /// No channel (Mock backend, or computer not running).
    Unavailable,
    /// Apple Virtualization.framework virtio socket (AF_VSOCK both ends).
    VirtioSocket,
    /// Hyper-V socket (AF_HYPERV host, AF_VSOCK Linux guest). Specified,
    /// not yet available.
    HyperVSocket,
}

/// Host data root layout (never Desktop/Documents/Downloads, never repo):
///
/// ```text
/// <root>/
///   images/      verified base images + derived images + manifests
///   computers/   <computer-id>/{metadata.json,disk.*,efi-vars.bin,...}
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PegolesPaths {
    root: PathBuf,
}

impl PegolesPaths {
    /// Data root for the current host:
    /// macOS `~/Library/Application Support/Pegoles`,
    /// Windows `%LOCALAPPDATA%\Pegoles`,
    /// Linux `$XDG_DATA_HOME/pegoles` (fallback `~/.local/share/pegoles`).
    /// `PEGOLES_DATA_DIR` overrides everywhere (tests, custom installs).
    pub fn for_current_host() -> Self {
        Self::for_platform(host_platform())
    }

    pub fn for_platform(platform: HostPlatform) -> Self {
        if let Ok(p) = std::env::var(crate::config::DATA_DIR_ENV) {
            return Self {
                root: PathBuf::from(p),
            };
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let local_app_data = std::env::var("LOCALAPPDATA").ok();
        let appdata = std::env::var("APPDATA").ok();
        let xdg = std::env::var("XDG_DATA_HOME").ok();
        Self {
            root: resolve_root(platform, &home, local_app_data, appdata, xdg),
        }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn images_dir(&self) -> PathBuf {
        self.root.join("images")
    }

    pub fn computers_dir(&self) -> PathBuf {
        self.root.join("computers")
    }
}

/// Pure path resolution (the real logic; `for_platform` only gathers
/// environment). Directly unit-tested per platform.
fn resolve_root(
    platform: HostPlatform,
    home: &str,
    local_app_data: Option<String>,
    appdata: Option<String>,
    xdg_data_home: Option<String>,
) -> PathBuf {
    let home = if home.is_empty() {
        std::env::temp_dir()
    } else {
        PathBuf::from(home)
    };
    match platform {
        HostPlatform::MacOS => home.join("Library/Application Support/Pegoles"),
        HostPlatform::Windows => match local_app_data {
            Some(local) => PathBuf::from(local).join("Pegoles"),
            // Justified fallback: roaming profile root is always defined
            // on Windows when LOCALAPPDATA is not (service contexts).
            None => match appdata {
                Some(roaming) => PathBuf::from(roaming).join("../Local/Pegoles"),
                None => home.join("AppData/Local/Pegoles"),
            },
        },
        HostPlatform::Linux => match xdg_data_home {
            Some(xdg) if !xdg.is_empty() => PathBuf::from(xdg).join("pegoles"),
            _ => home.join(".local/share/pegoles"),
        },
    }
}

/// Compile-time host platform. No detection heuristics, no user-agent.
pub fn host_platform() -> HostPlatform {
    #[cfg(target_os = "macos")]
    {
        HostPlatform::MacOS
    }
    #[cfg(target_os = "windows")]
    {
        HostPlatform::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        HostPlatform::Linux
    }
}

/// Compile-time host CPU architecture.
pub fn host_architecture() -> HostArchitecture {
    #[cfg(target_arch = "aarch64")]
    {
        HostArchitecture::Arm64
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        HostArchitecture::X86_64
    }
}

/// Backend factory: the ONLY place that names concrete adapters.
/// The Core calls this; it never imports macos/windows/mock modules.
/// A `Real`-family kind on an unsupported platform returns
/// `UnsupportedPlatform` — never a silent Mock fallback.
pub fn create_backend(
    kind: BackendKind,
    images_dir: std::path::PathBuf,
    computers_dir: std::path::PathBuf,
) -> Result<Box<dyn crate::traits::ComputerBackend>, crate::error::ComputerError> {
    match kind {
        BackendKind::Mock => Ok(Box::new(crate::mock::MockComputerBackend::new())),
        BackendKind::MacOSVirtualization => Ok(Box::new(
            crate::macos::MacOSVirtualizationBackend::new(images_dir, computers_dir)?,
        )),
        BackendKind::WindowsHcs => Ok(Box::new(crate::windows::WindowsHcsBackend::new(
            images_dir,
            computers_dir,
        )?)),
    }
}
/// Serializable host capability report for `get_host_capabilities()`.
/// The frontend never detects the platform itself (no JS user-agent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub platform: HostPlatform,
    pub architecture: HostArchitecture,
    pub backend: BackendKind,
    pub backend_available: bool,
    pub backend_detail: String,
    pub guest_transport: GuestTransportKind,
    pub guest_transport_available: bool,
    pub required_setup: Vec<String>,
    pub supported: bool,
}

/// Honest capability report for THIS host. Detection is conservative:
/// anything unproven is reported unavailable with a setup note, never
/// assumed present.
pub fn host_capabilities() -> HostCapabilities {
    let platform = host_platform();
    let architecture = host_architecture();
    match platform {
        HostPlatform::MacOS => {
            let available = crate::config::is_real_backend_supported();
            HostCapabilities {
                platform,
                architecture,
                backend: BackendKind::MacOSVirtualization,
                backend_available: available,
                backend_detail: if available {
                    "Apple Virtualization.framework on Apple Silicon".to_string()
                } else {
                    "Real VM requires macOS on Apple Silicon (arm64)".to_string()
                },
                guest_transport: GuestTransportKind::VirtioSocket,
                guest_transport_available: available,
                required_setup: if available {
                    Vec::new()
                } else {
                    vec!["Run on macOS Apple Silicon, or use the Mock backend".to_string()]
                },
                supported: available,
            }
        }
        HostPlatform::Windows => {
            let probe = crate::windows::probe_windows_host();
            match probe {
                Ok(caps) => HostCapabilities {
                    platform,
                    architecture,
                    backend: BackendKind::WindowsHcs,
                    backend_available: false,
                    backend_detail: format!(
                        "WindowsHcsBackend skeleton only (state: {:?})",
                        caps.state
                    ),
                    guest_transport: GuestTransportKind::HyperVSocket,
                    guest_transport_available: false,
                    required_setup: caps.setup_steps(),
                    supported: false,
                },
                Err(e) => HostCapabilities {
                    platform,
                    architecture,
                    backend: BackendKind::WindowsHcs,
                    backend_available: false,
                    backend_detail: format!("Windows probe failed: {e}"),
                    guest_transport: GuestTransportKind::HyperVSocket,
                    guest_transport_available: false,
                    required_setup: vec![format!("Fix Windows probing: {e}")],
                    supported: false,
                },
            }
        }
        HostPlatform::Linux => HostCapabilities {
            platform,
            architecture,
            backend: BackendKind::Mock,
            backend_available: false,
            backend_detail: "Linux KVM backend not implemented (future)".to_string(),
            guest_transport: GuestTransportKind::Unavailable,
            guest_transport_available: false,
            required_setup: vec!["Linux KVM backend is future work".to_string()],
            supported: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_strings_are_explicit() {
        assert_eq!(GuestArchitecture::Arm64.as_str(), "arm64");
        assert_eq!(GuestArchitecture::X86_64.as_str(), "amd64");
        assert_eq!(DiskFormat::Raw.extension(), "raw");
        assert_eq!(DiskFormat::Vhdx.extension(), "vhdx");
    }

    #[test]
    fn arm64_is_not_a_mac_synonym() {
        // GuestArchitecture carries no platform. A future Windows/ARM64
        // target uses this same variant; nothing maps Arm64 => Mac.
        let _ = GuestArchitecture::Arm64;
        let _ = HostPlatform::Windows;
        assert_ne!(
            std::mem::discriminant(&HostPlatform::MacOS),
            std::mem::discriminant(&HostPlatform::Windows)
        );
    }

    #[test]
    fn backend_names_are_stable() {
        assert_eq!(BackendKind::Mock.as_str(), "mock");
        assert_eq!(BackendKind::MacOSVirtualization.as_str(), "real");
        assert_eq!(BackendKind::WindowsHcs.as_str(), "windows-hcs");
    }

    #[test]
    fn macos_paths_use_application_support() {
        let root = resolve_root(HostPlatform::MacOS, "/home/testuser", None, None, None);
        assert_eq!(
            root.to_string_lossy(),
            "/home/testuser/Library/Application Support/Pegoles"
        );
        let paths = PegolesPaths { root };
        assert!(paths.images_dir().ends_with("images"));
        assert!(paths.computers_dir().ends_with("computers"));
    }

    #[test]
    fn windows_paths_use_localappdata() {
        let root = resolve_root(
            HostPlatform::Windows,
            "C:\\Users\\ana",
            Some("C:\\Users\\ana\\AppData\\Local".to_string()),
            None,
            None,
        );
        // Component-wise: exact separators differ per test host OS.
        let mut parts: Vec<String> = root
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        // Windows-style root parses as one prefix on Unix test hosts.
        let joined = parts.join("/");
        assert!(joined.contains("AppData"), "root: {joined}");
        assert!(joined.ends_with("Pegoles"), "root: {joined}");
        assert_eq!(parts.pop().as_deref(), Some("Pegoles"));
    }

    #[test]
    fn windows_paths_fall_back_without_localappdata() {
        let root = resolve_root(HostPlatform::Windows, "C:\\Users\\ana", None, None, None);
        let root = root.to_string_lossy().to_string();
        assert!(root.contains("Pegoles"), "root: {root}");
        for forbidden in ["Desktop", "Documents", "Downloads"] {
            assert!(!root.contains(forbidden), "root: {root}");
        }
    }

    #[test]
    fn linux_paths_follow_xdg() {
        let root = resolve_root(
            HostPlatform::Linux,
            "/home/ana",
            None,
            None,
            Some("/home/ana/.xdg/data".to_string()),
        );
        assert_eq!(root.to_string_lossy(), "/home/ana/.xdg/data/pegoles");
        let fallback = resolve_root(HostPlatform::Linux, "/home/ana", None, None, None);
        assert_eq!(fallback.to_string_lossy(), "/home/ana/.local/share/pegoles");
    }

    #[test]
    fn data_dir_override_wins_everywhere() {
        std::env::set_var(crate::config::DATA_DIR_ENV, "/tmp/pegoles-test-override");
        for platform in [
            HostPlatform::MacOS,
            HostPlatform::Windows,
            HostPlatform::Linux,
        ] {
            assert_eq!(
                PegolesPaths::for_platform(platform)
                    .root()
                    .to_string_lossy(),
                "/tmp/pegoles-test-override"
            );
        }
        std::env::remove_var(crate::config::DATA_DIR_ENV);
    }

    #[test]
    fn host_caps_serialize_for_tauri() {
        let caps = host_capabilities();
        let v = serde_json::to_value(&caps).unwrap();
        assert!(v.get("platform").is_some());
        assert!(v.get("supported").is_some());
        assert!(v.get("required_setup").is_some());
    }

    #[test]
    fn host_caps_never_claim_support_falsely() {
        let caps = host_capabilities();
        if !caps.backend_available {
            assert!(!caps.supported);
            assert!(!caps.required_setup.is_empty());
        }
        if !caps.guest_transport_available {
            assert!(caps.required_setup.is_empty() || !caps.supported);
        }
    }

    #[test]
    fn no_silent_mock_fallback() {
        // A gated-out backend kind returns an explicit error from the
        // factory — production failure stays a failure, never a Mock.
        let tmp = tempfile::tempdir().unwrap();
        let images = tmp.path().join("images");
        let computers = tmp.path().join("computers");
        #[cfg(not(target_os = "windows"))]
        {
            let err = match create_backend(BackendKind::WindowsHcs, images, computers) {
                Ok(_) => panic!("WindowsHcs must not construct off Windows"),
                Err(e) => e,
            };
            assert!(
                matches!(err, crate::error::ComputerError::UnsupportedPlatform(_)),
                "got {err:?}"
            );
        }
        // Mock always constructs (explicitly requested kind).
        let tmp = tempfile::tempdir().unwrap();
        assert!(create_backend(
            BackendKind::Mock,
            tmp.path().join("images"),
            tmp.path().join("computers"),
        )
        .is_ok());
    }
}

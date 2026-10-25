//! First-run onboarding: where the person is in it, and the system check
//! behind its "Check this computer" screen.
//!
//! The step is persisted in `<data>/onboarding.json`, so quitting, a crash
//! or a Windows restart (to turn on virtualization) resumes at the same
//! screen. The check reports facts and states; the webview turns them into
//! sentences (`src/onboarding/systemCheck.ts`) and keeps the technical
//! text for its "Technical details" disclosure only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "onboarding.json";
const STATE_VERSION: u32 = 1;
/// Largest onboarding file we read (it is a few hundred bytes).
const MAX_FILE_BYTES: u64 = 16 * 1024;

const GIB: u64 = 1 << 30;
/// Below this Pegoles Local and its computer do not fit side by side.
pub const MINIMUM_MEMORY_BYTES: u64 = 8 * GIB;
/// Comfortable: the model, the computer and a browser on the host.
pub const RECOMMENDED_MEMORY_BYTES: u64 = 16 * GIB;
/// Room the computer's disk grows into, plus temporary files while setting up.
pub const WORKING_SPACE_BYTES: u64 = 3 * GIB;

/// The onboarding screens, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Welcome,
    How,
    Check,
    Setup,
    Intelligence,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingState {
    pub version: u32,
    pub step: Step,
    pub completed: bool,
    /// Set when the person chose to restart Windows to finish enabling
    /// virtualization; the next launch resumes the check from there.
    #[serde(default)]
    pub restart_requested: bool,
}

impl OnboardingState {
    pub fn fresh() -> Self {
        Self {
            version: STATE_VERSION,
            step: Step::Welcome,
            completed: false,
            restart_requested: false,
        }
    }

    pub fn finished() -> Self {
        Self {
            completed: true,
            step: Step::Ready,
            ..Self::fresh()
        }
    }
}

pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// The saved state, or — when there is none — a new one. Someone who set
/// Pegoles up before onboarding existed (model and computer image already
/// in place) is not sent through it again.
pub fn load(data_dir: &Path, already_set_up: impl FnOnce() -> bool) -> OnboardingState {
    match read(&state_path(data_dir)) {
        Some(state) => state,
        None if already_set_up() => OnboardingState::finished(),
        None => OnboardingState::fresh(),
    }
}

/// The page's first render knows whether to open onboarding (no flash of
/// the main window, no wait): a frozen `window.__PEGOLES_BOOT__` set before
/// any app script runs. Only enums and booleans; nothing secret.
pub fn boot_script(data_dir: &Path, already_set_up: impl FnOnce() -> bool) -> String {
    let state = load(data_dir, already_set_up);
    let json = serde_json::to_string(&state).unwrap_or_else(|_| "null".into());
    format!("Object.defineProperty(window,'__PEGOLES_BOOT__',{{value:Object.freeze({{onboarding:Object.freeze({json})}}),writable:false,configurable:false}});")
}

fn read(path: &Path) -> Option<OnboardingState> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let state: OnboardingState = serde_json::from_str(&text).ok()?;
    (state.version == STATE_VERSION).then_some(state)
}

/// Atomic write (temporary file + rename), owner-only on Unix.
pub fn save(data_dir: &Path, state: &OnboardingState) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("could not save setup progress: {e}"))?;
    let path = state_path(data_dir);
    let tmp = data_dir.join(format!("{FILE_NAME}.tmp"));
    let body = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|e| format!("could not save setup progress: {e}"))?;
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("could not save setup progress: {e}"))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not save setup progress: {e}"))
}

// ── System check ─────────────────────────────────────────────────────

/// Whether this computer can give Pegoles its own isolated computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualizationState {
    Ready,
    /// A Windows feature must be turned on (administrator, then a restart).
    NeedsEnable,
    /// Turned on; Windows has to restart once before it can be used.
    RestartPending,
    /// Hardware virtualization is off in the firmware (BIOS/UEFI).
    FirmwareDisabled,
    /// This OS or processor cannot run Pegoles' computer.
    Unsupported,
    /// Could not be determined; setup will find out.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Virtualization {
    pub state: VirtualizationState,
    /// Pegoles can fix it itself (after asking for administrator approval).
    pub fixable: bool,
    /// Plain facts for "Technical details" (feature names, codes).
    pub technical: String,
}

/// How Pegoles Local will run its model on this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccelerationKind {
    /// Apple silicon GPU through Metal.
    Metal,
    /// NVIDIA GPU through CUDA.
    Cuda,
    /// A GPU through Vulkan (NVIDIA, AMD or Intel).
    Vulkan,
    /// The processor only (works, slower).
    Cpu,
    /// No supported way to run the model here.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Acceleration {
    pub kind: AccelerationKind,
    /// The device, as the OS names it ("Apple M4 Pro", "NVIDIA GeForce RTX 4060").
    pub device: Option<String>,
    pub technical: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemCheck {
    /// `macos` | `windows` | `linux`.
    pub platform: &'static str,
    /// "macOS 15.5", "Windows 11 Home (24H2)".
    pub os_name: String,
    pub os_supported: bool,
    /// What the OS must be at least, in words ("macOS 14", "Windows 11").
    pub os_minimum: &'static str,
    /// `arm64` | `x86_64`.
    pub architecture: &'static str,
    pub architecture_supported: bool,
    pub virtualization: Virtualization,
    pub memory_bytes: u64,
    pub memory_minimum_bytes: u64,
    pub memory_recommended_bytes: u64,
    /// Free space where Pegoles keeps its data; None when unknown.
    pub disk_free_bytes: Option<u64>,
    /// What the remaining setup still needs on disk (0 when all is in place).
    pub disk_needed_bytes: u64,
    pub acceleration: Acceleration,
    /// The Pegoles Local runtime shipped with the app is present.
    pub runtime_ready: bool,
    pub runtime_problem: Option<String>,
    /// Already set up: the model is installed, the computer image is ready.
    pub model_ready: bool,
    pub image_ready: bool,
}

/// What is still missing on disk for setup to complete.
pub fn disk_needed(
    model_ready: bool,
    model_bytes: u64,
    image_ready: bool,
    image_bytes: u64,
) -> u64 {
    let mut needed = 0;
    if !model_ready {
        needed += model_bytes;
    }
    if !image_ready {
        needed += image_bytes;
    }
    if needed > 0 {
        needed += WORKING_SPACE_BYTES;
    }
    needed
}

/// Facts the platform layer gathers; `check` combines them with setup state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFacts {
    pub platform: &'static str,
    pub os_name: String,
    pub os_supported: bool,
    pub os_minimum: &'static str,
    pub architecture: &'static str,
    pub architecture_supported: bool,
    pub virtualization: Virtualization,
    pub memory_bytes: u64,
    pub acceleration: Acceleration,
}

pub struct SetupFacts {
    pub runtime_ready: bool,
    pub runtime_problem: Option<String>,
    pub model_ready: bool,
    pub model_bytes: u64,
    pub image_ready: bool,
    pub image_bytes: u64,
    pub disk_free_bytes: Option<u64>,
}

pub fn check(host: HostFacts, setup: SetupFacts) -> SystemCheck {
    SystemCheck {
        platform: host.platform,
        os_name: host.os_name,
        os_supported: host.os_supported,
        os_minimum: host.os_minimum,
        architecture: host.architecture,
        architecture_supported: host.architecture_supported,
        virtualization: host.virtualization,
        memory_bytes: host.memory_bytes,
        memory_minimum_bytes: MINIMUM_MEMORY_BYTES,
        memory_recommended_bytes: RECOMMENDED_MEMORY_BYTES,
        disk_free_bytes: setup.disk_free_bytes,
        disk_needed_bytes: disk_needed(
            setup.model_ready,
            setup.model_bytes,
            setup.image_ready,
            setup.image_bytes,
        ),
        acceleration: host.acceleration,
        runtime_ready: setup.runtime_ready,
        runtime_problem: setup.runtime_problem,
        model_ready: setup.model_ready,
        image_ready: setup.image_ready,
    }
}

/// What happened when Pegoles tried to turn virtualization on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixOutcome {
    /// Done; usable now.
    Ready,
    /// Done; Windows must restart once before it can be used.
    RestartRequired,
    /// The person declined the administrator prompt. Nothing changed.
    Declined,
}

/// Turn on the Windows feature Pegoles' computer needs (elevated helper).
pub fn fix_virtualization() -> Result<FixOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        use pegoles_computer::windows::EnableOutcome;
        match pegoles_computer::windows::enable_virtualization_elevated() {
            Ok(EnableOutcome::Enabled) => Ok(FixOutcome::Ready),
            Ok(EnableOutcome::RestartRequired) => Ok(FixOutcome::RestartRequired),
            Ok(EnableOutcome::Declined) => Ok(FixOutcome::Declined),
            Err(e) => Err(e.to_string()),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("nothing to turn on: this computer's virtualization is built in".into())
    }
}

/// Restart the host (Windows only, after explicit consent in the UI).
pub fn restart_host() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        pegoles_computer::windows::restart_for_setup().map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("a restart is only needed on Windows".into())
    }
}

/// This computer, as far as the OS tells us. Never guesses: unknowns stay
/// `Unknown` and setup finds out for real.
pub fn host_facts() -> HostFacts {
    platform::facts()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    fn product_version() -> Option<String> {
        let hw = pegoles_inference::hardware::os_product_version();
        hw.filter(|v| !v.is_empty())
    }

    fn major(version: &str) -> Option<u32> {
        version.split('.').next()?.parse().ok()
    }

    pub fn facts() -> HostFacts {
        let hw = pegoles_inference::hardware::detect();
        let version = product_version();
        let os_supported = version.as_deref().and_then(major).is_some_and(|m| m >= 14);
        let arm64 = cfg!(target_arch = "aarch64");
        let virtualization = if arm64 && os_supported {
            Virtualization {
                state: VirtualizationState::Ready,
                fixable: false,
                technical: "Apple Virtualization framework (built into macOS)".into(),
            }
        } else {
            Virtualization {
                state: VirtualizationState::Unsupported,
                fixable: false,
                technical: format!(
                    "Pegoles' computer needs macOS 14 or later on Apple silicon (this Mac: {} {})",
                    version.as_deref().unwrap_or("unknown version"),
                    std::env::consts::ARCH
                ),
            }
        };
        HostFacts {
            platform: "macos",
            os_name: match &version {
                Some(v) => format!("macOS {v}"),
                None => "macOS".into(),
            },
            os_supported,
            os_minimum: "macOS 14",
            architecture: if arm64 { "arm64" } else { "x86_64" },
            architecture_supported: arm64,
            virtualization,
            memory_bytes: hw.total_memory_bytes,
            acceleration: if hw.apple_silicon {
                Acceleration {
                    kind: AccelerationKind::Metal,
                    device: hw.chip.clone(),
                    technical: "Metal (Apple silicon GPU), MLX runtime".into(),
                }
            } else {
                Acceleration {
                    kind: AccelerationKind::None,
                    device: hw.chip.clone(),
                    technical: "Pegoles Local needs Apple silicon on macOS".into(),
                }
            },
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub fn facts() -> HostFacts {
        let hw = pegoles_inference::hardware::detect();
        let readiness = pegoles_computer::windows::readiness();
        let x64 = cfg!(target_arch = "x86_64");
        let accel = pegoles_inference::hardware::windows_acceleration();
        HostFacts {
            platform: "windows",
            os_name: readiness.os_name.clone(),
            os_supported: readiness.os_supported,
            os_minimum: "Windows 11",
            architecture: if x64 { "x86_64" } else { "arm64" },
            architecture_supported: x64,
            virtualization: Virtualization {
                state: match readiness.state {
                    pegoles_computer::windows::VirtualizationReadiness::Ready => {
                        VirtualizationState::Ready
                    }
                    pegoles_computer::windows::VirtualizationReadiness::NeedsEnable => {
                        VirtualizationState::NeedsEnable
                    }
                    pegoles_computer::windows::VirtualizationReadiness::RestartPending => {
                        VirtualizationState::RestartPending
                    }
                    pegoles_computer::windows::VirtualizationReadiness::FirmwareDisabled => {
                        VirtualizationState::FirmwareDisabled
                    }
                    pegoles_computer::windows::VirtualizationReadiness::Unsupported => {
                        VirtualizationState::Unsupported
                    }
                    pegoles_computer::windows::VirtualizationReadiness::Unknown => {
                        VirtualizationState::Unknown
                    }
                },
                fixable: readiness.fixable,
                technical: readiness.technical.clone(),
            },
            memory_bytes: hw.total_memory_bytes,
            acceleration: Acceleration {
                kind: match accel.kind {
                    pegoles_inference::hardware::AcceleratorKind::Cuda => AccelerationKind::Cuda,
                    pegoles_inference::hardware::AcceleratorKind::Vulkan => {
                        AccelerationKind::Vulkan
                    }
                    pegoles_inference::hardware::AcceleratorKind::Metal => AccelerationKind::Metal,
                    pegoles_inference::hardware::AcceleratorKind::Cpu => AccelerationKind::Cpu,
                },
                device: accel.device,
                technical: accel.technical,
            },
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn facts() -> HostFacts {
        let hw = pegoles_inference::hardware::detect();
        HostFacts {
            platform: "linux",
            os_name: "Linux".into(),
            os_supported: false,
            os_minimum: "macOS 14 or Windows 11",
            architecture: if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            },
            architecture_supported: false,
            virtualization: Virtualization {
                state: VirtualizationState::Unsupported,
                fixable: false,
                technical: "Pegoles has no Linux host backend yet".into(),
            },
            memory_bytes: hw.total_memory_bytes,
            acceleration: Acceleration {
                kind: AccelerationKind::None,
                device: None,
                technical: "no Linux inference backend yet".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pegoles-onboarding-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn new_people_start_at_welcome_and_returning_ones_skip() {
        let dir = tmp("fresh");
        assert_eq!(load(&dir, || false), OnboardingState::fresh());
        assert_eq!(load(&dir, || true), OnboardingState::finished());
    }

    #[test]
    fn progress_survives_a_restart() {
        let dir = tmp("save");
        let state = OnboardingState {
            step: Step::Check,
            restart_requested: true,
            ..OnboardingState::fresh()
        };
        save(&dir, &state).unwrap();
        // A saved state wins over "already set up".
        assert_eq!(load(&dir, || true), state);
        assert!(!dir.join("onboarding.json.tmp").exists());
    }

    #[test]
    fn unreadable_or_foreign_files_start_over() {
        let dir = tmp("bad");
        std::fs::write(state_path(&dir), "not json").unwrap();
        assert_eq!(load(&dir, || false), OnboardingState::fresh());
        std::fs::write(
            state_path(&dir),
            r#"{"version":99,"step":"ready","completed":true}"#,
        )
        .unwrap();
        assert_eq!(load(&dir, || false), OnboardingState::fresh());
        std::fs::write(state_path(&dir), vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert_eq!(load(&dir, || false), OnboardingState::fresh());
    }

    #[test]
    fn boot_script_carries_only_the_state() {
        let dir = tmp("boot");
        let script = boot_script(&dir, || false);
        assert!(script.starts_with("Object.defineProperty(window,'__PEGOLES_BOOT__'"));
        assert!(script.contains(r#""step":"welcome""#));
        assert!(script.contains(r#""completed":false"#));
    }

    #[test]
    fn disk_needed_counts_only_what_is_missing() {
        assert_eq!(disk_needed(true, 10, true, 20), 0);
        assert_eq!(disk_needed(false, 10, true, 20), 10 + WORKING_SPACE_BYTES);
        assert_eq!(disk_needed(false, 10, false, 20), 30 + WORKING_SPACE_BYTES);
    }

    #[test]
    fn check_carries_thresholds_and_setup_state() {
        let host = HostFacts {
            platform: "macos",
            os_name: "macOS 15.5".into(),
            os_supported: true,
            os_minimum: "macOS 14",
            architecture: "arm64",
            architecture_supported: true,
            virtualization: Virtualization {
                state: VirtualizationState::Ready,
                fixable: false,
                technical: String::new(),
            },
            memory_bytes: 16 * GIB,
            acceleration: Acceleration {
                kind: AccelerationKind::Metal,
                device: Some("Apple M4".into()),
                technical: String::new(),
            },
        };
        let c = check(
            host,
            SetupFacts {
                runtime_ready: true,
                runtime_problem: None,
                model_ready: false,
                model_bytes: 2 * GIB,
                image_ready: true,
                image_bytes: GIB,
                disk_free_bytes: Some(50 * GIB),
            },
        );
        assert_eq!(c.memory_minimum_bytes, MINIMUM_MEMORY_BYTES);
        assert_eq!(c.disk_needed_bytes, 2 * GIB + WORKING_SPACE_BYTES);
        assert!(!c.model_ready && c.image_ready);
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["virtualization"]["state"], "ready");
        assert_eq!(json["acceleration"]["kind"], "metal");
    }

    #[test]
    fn this_host_reports_something_honest() {
        let facts = host_facts();
        assert!(!facts.os_name.is_empty());
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(facts.acceleration.kind, AccelerationKind::Metal);
        }
    }
}

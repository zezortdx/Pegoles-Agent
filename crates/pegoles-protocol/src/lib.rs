//! Shared protocol types for Pegoles Agent.
//!
//! These types travel (today locally, later over the network) between:
//! desktop <-> core, mobile <-> host, core <-> computer, core <-> guest runtime.
//!
//! SECURITY INVARIANT: all agent behavior is expressed as [`ComputerAction`].
//! There is intentionally NO `HostShell` / `ExecuteOnHost` / `RawHostCommand`
//! variant. Shell/file actions always mean "inside Pegoles Computer", never the host.

pub mod actions;
pub mod computer;
pub mod display;
pub mod events;
pub mod guest;
pub mod ids;
pub mod keys;
pub mod limits;
pub mod policy;
pub mod tasks;

pub use actions::{ActionOutcome, ActionRequest, ActionResult, ComputerAction, PointerButton};
pub use computer::{ComputerConfig, ComputerInfo, ComputerState, SnapshotId, VirtualPath};
pub use display::{
    ControlOwner, DisplayConfig, DisplayConfigError, DisplayProfile, FrameEncoding,
    GraphicalSessionState, ObservedFrameMeta, ViewportState,
};
pub use events::AgentEvent;
pub use guest::GuestRuntimeState;
pub use ids::{ActionId, AgentId, ComputerId, FrameId, SessionId, TaskId};
pub use keys::{is_modifier, normalize_key_name, validate_chord};
pub use policy::{
    ApprovalDecision, ApprovalRequest, Capability, Decision, Permission, PolicyVerdict, RiskLevel,
};
pub use tasks::{AgentTask, TaskStatus};

/// Architecture tripwire (Phase 3.5): the shared-vocabulary crates
/// (protocol, guest-proto, policy) must stay platform-free: no hypervisor
/// names, no OS conditionals, no native types — not even in comments.
/// (Host-path *denylists* like `C:\` in policy are the documented
/// exception: they recognize host paths to DENY them; see
/// PORTABILITY_AUDIT.md. The token list below bans framework names,
/// not path literals.)
/// Files defining a tripwire are exempt by construction (they name the
/// tokens to forbid).
#[cfg(test)]
mod portability_tests {
    #[test]
    fn shared_crates_stay_platform_free() {
        for dir in [
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pegoles-guest-proto/src"),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pegoles-policy/src"),
        ] {
            let mut files = Vec::new();
            collect_rs(&dir, &mut files);
            assert!(!files.is_empty(), "no sources under {}", dir.display());
            for path in files {
                let src = std::fs::read_to_string(&path).unwrap();
                if src.contains("portability_tests") {
                    continue;
                }
                let src = src.to_lowercase();
                for token in [
                    "target_os",
                    "target_arch",
                    "virtualization",
                    // NOTE: bare "hyperv" is deliberately absent: it collides
                    // with the generic word "hypervisor", which shared crates
                    // must be free to use. "hyper-v" still catches refs.
                    "hyper-v",
                    "hcs",
                    "swift",
                    "appkit",
                    "win32",
                    "winsock",
                    "powershell",
                    "wmi",
                ] {
                    assert!(
                        !src.contains(token),
                        "{} leaks platform detail: {token}",
                        path.display()
                    );
                }
            }
        }
    }

    fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
}

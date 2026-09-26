//! HCS bindings: direct ComputeCore.dll FFI, no PowerShell, no WMI scripts.
//!
//! Signatures verified against Microsoft Learn (Host Compute System API
//! Reference; HcsCreate/Start/TerminateComputeSystem pages; Compute System
//! Samples). Behavior is UNVERIFIED without Windows hardware — Windows CI
//! checks compilation; `PEGOLES_REAL_WINDOWS_VM_TEST=1` checks behavior.
//!
//! Design notes:
//! - Handles are RAII (`HcsSystem` closes on drop); operations run through
//!   `run_operation` (create op -> call -> wait -> close op).
//! - Native HRESULT codes are preserved in every error (§25: context kept,
//!   user-facing mapping happens in pegoles-computer error → UI strings).
//! - Event callbacks: registered where cheap, but state truth ALWAYS comes
//!   from re-queried properties (HCS_EVENT layout is intentionally NOT
//!   bound here — no fabricated struct layouts).

use std::ffi::c_void;

/// Opaque HCS handles (never exposed outside this adapter).
/// Live on Windows (FFI surface); kept compiled everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub type HcsSystem = *mut c_void;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub type HcsOperation = *mut c_void;
/// Windows HRESULT (S_OK == 0).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub type Hresult = i32;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub const S_OK: Hresult = 0;
/// Infinite wait for HcsWaitForOperationResult.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub const HCS_WAIT_INFINITE: u32 = 0xFFFF_FFFF;

#[cfg(windows)]
#[link(name = "computecore")]
extern "C" {
    fn HcsCreateComputeSystem(
        id: *const u16,
        configuration: *const u16,
        operation: HcsOperation,
        security_descriptor: *const c_void,
        compute_system: *mut HcsSystem,
    ) -> Hresult;
    fn HcsOpenComputeSystem(
        id: *const u16,
        requested_access: u32,
        compute_system: *mut HcsSystem,
    ) -> Hresult;
    fn HcsCloseComputeSystem(compute_system: HcsSystem);
    fn HcsStartComputeSystem(
        compute_system: HcsSystem,
        operation: HcsOperation,
        options: *const u16,
    ) -> Hresult;
    fn HcsShutDownComputeSystem(
        compute_system: HcsSystem,
        operation: HcsOperation,
        options: *const u16,
    ) -> Hresult;
    fn HcsTerminateComputeSystem(
        compute_system: HcsSystem,
        operation: HcsOperation,
        options: *const u16,
    ) -> Hresult;
    fn HcsPauseComputeSystem(
        compute_system: HcsSystem,
        operation: HcsOperation,
        options: *const u16,
    ) -> Hresult;
    fn HcsResumeComputeSystem(
        compute_system: HcsSystem,
        operation: HcsOperation,
        options: *const u16,
    ) -> Hresult;
    fn HcsGetComputeSystemProperties(
        compute_system: HcsSystem,
        operation: HcsOperation,
        property_query: *const u16,
    ) -> Hresult;
    fn HcsCreateOperation(
        context: *const c_void,
        callback: Option<extern "system" fn(HcsOperation, *mut c_void)>,
    ) -> HcsOperation;
    fn HcsCloseOperation(operation: HcsOperation);
    fn HcsWaitForOperationResult(
        operation: HcsOperation,
        timeout_ms: u32,
        result_document: *mut *mut u16,
    ) -> Hresult;
    fn HcsSetComputeSystemCallback(
        compute_system: HcsSystem,
        callback_options: u64,
        context: *const c_void,
        callback: Option<extern "system" fn(HcsSystem, *const c_void, *mut c_void)>,
    ) -> Hresult;
}

#[cfg(windows)]
#[link(name = "ole32")]
extern "C" {
    fn CoTaskMemFree(pv: *mut c_void);
}

/// Native HCS failure with the HRESULT preserved for logs.
/// Live on Windows (session errors); kept compiled everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HcsError {
    pub code: Hresult,
    pub operation: &'static str,
    pub detail: String,
}

impl std::fmt::Display for HcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} failed: HRESULT {:#010X} ({})",
            self.operation, self.code as u32, self.detail
        )
    }
}

impl std::error::Error for HcsError {}

/// Live on Windows (session results); kept compiled everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub type HcsResult<T> = Result<T, HcsError>;

/// High-level session operations. Each awaits its HCS operation result
/// before returning, so callers report post-condition state, never
/// "the API call returned". All `#[cfg(windows)]`: elsewhere the
/// helper answers `not_implemented` without touching this module.
#[cfg(windows)]
pub use session::{
    create_compute_system, pause_compute_system, query_power_state, resume_compute_system,
    shutdown_compute_system, start_compute_system, terminate_compute_system,
};

#[cfg(windows)]
mod session {
    use super::*;

    fn utf16_len0(ptr: *const u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        unsafe {
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
        }
    }

    pub fn create_compute_system(id: &str, config_json: &str) -> Result<HcsSystemHandle, String> {
        let id_w = wide(id);
        let cfg_w = wide(config_json);
        let mut raw: HcsSystem = std::ptr::null_mut();
        let hr = run_operation("HcsCreateComputeSystem", HCS_WAIT_INFINITE, |op| unsafe {
            HcsCreateComputeSystem(
                id_w.as_ptr(),
                cfg_w.as_ptr(),
                op,
                std::ptr::null(),
                &mut raw,
            )
        });
        match hr {
            Ok(()) if !raw.is_null() => Ok(unsafe { HcsSystemHandle::take(raw) }),
            Ok(()) => Err("HCS returned success with a null system handle".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    macro_rules! op0 {
        ($name:ident, $ffi:ident) => {
            pub fn $name(system: &HcsSystemHandle) -> Result<(), String> {
                run_operation(stringify!($ffi), HCS_WAIT_INFINITE, |op| unsafe {
                    $ffi(system.raw(), op, std::ptr::null())
                })
                .map(|_| ())
                .map_err(|e| e.to_string())
            }
        };
    }

    op0!(start_compute_system, HcsStartComputeSystem);
    op0!(shutdown_compute_system, HcsShutDownComputeSystem);
    op0!(terminate_compute_system, HcsTerminateComputeSystem);
    op0!(resume_compute_system, HcsResumeComputeSystem);

    pub fn pause_compute_system(system: &HcsSystemHandle) -> Result<(), String> {
        let opt_w = wide(&crate::config::pause_options_json());
        run_operation("HcsPauseComputeSystem", HCS_WAIT_INFINITE, |op| unsafe {
            HcsPauseComputeSystem(system.raw(), op, opt_w.as_ptr())
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Best-effort power-state read. Returns the raw state string when the
    /// properties document carries a recognizable one, else None (caller
    /// keeps its cached state). Exact schema values get pinned against
    /// real hardware; until then only conservative matches apply.
    pub fn query_power_state(system: &HcsSystemHandle, query_json: &str) -> Option<String> {
        let q_w = wide(query_json);
        let doc = run_operation("HcsGetComputeSystemProperties", 30_000, |op| unsafe {
            HcsGetComputeSystemProperties(system.raw(), op, q_w.as_ptr())
        })
        .ok()??;
        super::parse_power_state(&doc)
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RAII compute-system handle. Drop closes WITHOUT terminating: HCS VMs
/// survive handle close (unlike macOS process-owned VMs), which is exactly
/// why Pegoles tracks identity separately from the native instance.
#[cfg(windows)]
pub struct HcsSystemHandle {
    raw: HcsSystem,
}

#[cfg(windows)]
impl HcsSystemHandle {
    /// Take ownership of a raw handle from create/open.
    /// SAFETY: must be a valid open HCS_SYSTEM exactly once.
    pub unsafe fn take(raw: HcsSystem) -> Self {
        Self { raw }
    }

    pub fn raw(&self) -> HcsSystem {
        self.raw
    }
}

#[cfg(windows)]
impl Drop for HcsSystemHandle {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { HcsCloseComputeSystem(self.raw) };
        }
    }
}

// SAFETY: HCS handles are thread-safe OS objects; the helper serializes
// lifecycle calls per computer and only shares the handle across its own
// command thread and the vsock pump (which never touches it).
#[cfg(windows)]
unsafe impl Send for HcsSystemHandle {}
#[cfg(windows)]
unsafe impl Sync for HcsSystemHandle {}

/// Run `body` inside a fresh HCS operation: create -> call -> wait with
/// timeout -> close. Returns the result document (JSON, caller parses).
#[cfg(windows)]
pub fn run_operation(
    op_name: &'static str,
    timeout_ms: u32,
    body: impl FnOnce(HcsOperation) -> Hresult,
) -> HcsResult<Option<String>> {
    unsafe {
        let op = HcsCreateOperation(std::ptr::null(), None);
        if op.is_null() {
            return Err(HcsError {
                code: -1,
                operation: "HcsCreateOperation",
                detail: "null operation handle".to_string(),
            });
        }
        let hr = body(op);
        if hr != S_OK {
            HcsCloseOperation(op);
            return Err(HcsError {
                code: hr,
                operation: op_name,
                detail: "call rejected".to_string(),
            });
        }
        let mut doc: *mut u16 = std::ptr::null_mut();
        let hr = HcsWaitForOperationResult(op, timeout_ms, &mut doc);
        let out = if hr == S_OK && !doc.is_null() {
            let mut len = 0usize;
            while *doc.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(doc, len));
            CoTaskMemFree(doc as *mut c_void);
            Some(text)
        } else {
            None
        };
        HcsCloseOperation(op);
        if hr != S_OK {
            return Err(HcsError {
                code: hr,
                operation: op_name,
                detail: "operation result failed".to_string(),
            });
        }
        Ok(out)
    }
}

/// Pure state-string extraction, tested on every OS with illustrative
/// documents (shape per HCS properties docs; exact production values get
/// pinned on hardware). Used by the Windows session; unit-tested here.
#[cfg_attr(all(not(target_os = "windows"), not(test)), allow(dead_code))]
pub fn parse_power_state(doc: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(doc).ok()?;
    for pointer in ["/State", "/RuntimeState", "/Properties/State"] {
        if let Some(s) = v.pointer(pointer).and_then(|s| s.as_str()) {
            let lower = s.to_lowercase();
            if ["running", "paused", "stopped", "created"].contains(&lower.as_str()) {
                return Some(lower);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn hresult_ok_is_zero() {
        assert_eq!(super::S_OK, 0);
        assert_eq!(super::HCS_WAIT_INFINITE, 0xFFFF_FFFF);
    }

    #[test]
    fn power_state_parsing_is_conservative() {
        use super::parse_power_state as parse;
        assert_eq!(parse(r#"{"State":"Running"}"#), Some("running".to_string()));
        assert_eq!(
            parse(r#"{"Properties":{"State":"Paused"}}"#),
            Some("paused".to_string())
        );
        // Unknown shapes keep the cache: None, never a guess.
        assert_eq!(parse(r#"{"State":"Migrating"}"#), None);
        assert_eq!(parse(r#"{"nope":1}"#), None);
        assert_eq!(parse("garbage"), None);
    }
}

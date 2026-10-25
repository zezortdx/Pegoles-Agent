//! Host Compute System calls. Every call runs inside its own HCS
//! operation and waits for the operation's result (bounded), so state is
//! reported from what HCS did, never from "the call returned". HRESULTs
//! are kept in every error for Technical details.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::HostComputeSystem::{
    HcsCloseComputeSystem, HcsCloseOperation, HcsCreateComputeSystem, HcsCreateOperation,
    HcsGetComputeSystemProperties, HcsGrantVmAccess, HcsPauseComputeSystem, HcsResumeComputeSystem,
    HcsRevokeVmAccess, HcsShutDownComputeSystem, HcsStartComputeSystem, HcsTerminateComputeSystem,
    HcsWaitForOperationResult, HCS_OPERATION, HCS_SYSTEM,
};

use super::util::{from_pwstr, local_free, pcwstr, wide};

/// Longest wait for one HCS operation.
const OPERATION_TIMEOUT_MS: u32 = 120_000;
const SHUTDOWN_TIMEOUT_MS: u32 = 30_000;

/// An open compute system, closed exactly once.
pub struct System(HCS_SYSTEM);

// SAFETY: HCS handles are thread-safe; the broker uses each from one session.
unsafe impl Send for System {}

impl Drop for System {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it once.
        unsafe { HcsCloseComputeSystem(self.0) };
    }
}

fn describe(call: &str, error: windows::core::Error) -> String {
    format!(
        "{call} failed: HRESULT {:#010X} ({})",
        error.code().0 as u32,
        error.message()
    )
}

/// Run `body` inside a fresh operation and wait for its result document.
fn operation(
    call: &str,
    timeout_ms: u32,
    body: impl FnOnce(HCS_OPERATION) -> windows::core::Result<()>,
) -> Result<String, String> {
    // SAFETY: no context, no callback: a waitable operation.
    let op = unsafe { HcsCreateOperation(None, None) };
    if op.is_invalid() {
        return Err(format!("{call}: cannot create an HCS operation"));
    }
    let result = body(op).map_err(|e| describe(call, e)).and_then(|()| {
        let mut document = PWSTR::null();
        // SAFETY: valid operation; the document is LocalAlloc'd by HCS.
        let waited = unsafe { HcsWaitForOperationResult(op, timeout_ms, Some(&mut document)) };
        // SAFETY: NUL-terminated (or null) document, freed once.
        let text = unsafe { from_pwstr(document) };
        unsafe { local_free(document.0.cast()) };
        match waited {
            Ok(()) => Ok(text),
            Err(e) => Err(format!(
                "{} {}",
                describe(call, e),
                text.chars().take(400).collect::<String>()
            )),
        }
    });
    // SAFETY: we created this operation and close it once.
    unsafe { HcsCloseOperation(op) };
    result
}

pub fn create(id: &str, document: &str) -> Result<System, String> {
    let id = wide(id);
    let document = wide(document);
    let mut system = HCS_SYSTEM::default();
    operation("HcsCreateComputeSystem", OPERATION_TIMEOUT_MS, |op| {
        // SAFETY: NUL-terminated id and document; no security descriptor
        // (reserved by the API).
        system = unsafe { HcsCreateComputeSystem(pcwstr(&id), pcwstr(&document), op, None)? };
        Ok(())
    })?;
    if system.is_invalid() {
        return Err("HcsCreateComputeSystem returned no system".into());
    }
    Ok(System(system))
}

macro_rules! simple {
    ($name:ident, $call:ident, $timeout:expr) => {
        pub fn $name(system: &System) -> Result<(), String> {
            operation(stringify!($call), $timeout, |op| {
                // SAFETY: an open system and a fresh operation; no options.
                unsafe { $call(system.0, op, PCWSTR::null()) }
            })
            .map(|_| ())
        }
    };
}

simple!(start, HcsStartComputeSystem, OPERATION_TIMEOUT_MS);
simple!(terminate, HcsTerminateComputeSystem, OPERATION_TIMEOUT_MS);
simple!(resume, HcsResumeComputeSystem, OPERATION_TIMEOUT_MS);
simple!(shutdown, HcsShutDownComputeSystem, SHUTDOWN_TIMEOUT_MS);

pub fn pause(system: &System) -> Result<(), String> {
    let options = wide(r#"{"SuspensionLevel":"Suspend"}"#);
    operation("HcsPauseComputeSystem", OPERATION_TIMEOUT_MS, |op| {
        // SAFETY: an open system, a fresh operation, NUL-terminated options.
        unsafe { HcsPauseComputeSystem(system.0, op, pcwstr(&options)) }
    })
    .map(|_| ())
}

/// Basic properties (RuntimeId, State) as HCS reports them.
pub fn properties(system: &System) -> Result<serde_json::Value, String> {
    let text = operation(
        "HcsGetComputeSystemProperties",
        OPERATION_TIMEOUT_MS,
        |op| {
            // SAFETY: an open system and a fresh operation; no query = basics.
            unsafe { HcsGetComputeSystemProperties(system.0, op, PCWSTR::null()) }
        },
    )?;
    serde_json::from_str(&text).map_err(|_| "HCS returned unreadable properties".into())
}

/// Let the VM worker open `path` (called while impersonating the user,
/// so it only works for files the user controls).
pub fn grant_vm_access(id: &str, path: &str) -> Result<(), String> {
    let id = wide(id);
    let path_w = wide(path);
    // SAFETY: NUL-terminated strings.
    unsafe { HcsGrantVmAccess(pcwstr(&id), pcwstr(&path_w)) }
        .map_err(|e| describe("HcsGrantVmAccess", e))
}

pub fn revoke_vm_access(id: &str, path: &str) {
    let id = wide(id);
    let path_w = wide(path);
    // SAFETY: NUL-terminated strings; failure only means nothing to revoke.
    let _ = unsafe { HcsRevokeVmAccess(pcwstr(&id), pcwstr(&path_w)) };
}

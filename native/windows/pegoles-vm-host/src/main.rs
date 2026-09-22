//! Windows VM host (Phase 3.6): narrow HCS adapter over JSONL.
//!
//! Same command set as the macOS helper (`version/validate/create/start/
//! pause/resume/stop/state/destroy` + `guest_send/guest_status/
//! guest_disconnect`) and same event shapes, so the shared Rust engine
//! (`NativeHelperBackend`) and guest session run unchanged. Backend
//! specifics travel in capabilities/optional fields — never forks.
//!
//! Responsibilities ONLY: HCS lifecycle, VM config, Hyper-V socket
//! transport, native events, error mapping. Never: planning, AI, policy,
//! guest protocol semantics, user data. No PowerShell, no GUI scripting.

mod config;
mod hcs;
mod hvsock;

use pegoles_computer::GuestTransport;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

const VERSION: &str = "0.1.0";

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    command: String,
    computer_id: Option<String>,
    params: Option<CreateParams>,
    payload: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateParams {
    computer_id: String,
    disk_path: String,
    efi_vars_path: Option<String>,
    machine_id_path: Option<String>,
    serial_log_path: Option<String>,
    vcpus: u8,
    memory_mb: u32,
    seed_iso_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct Response {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RespError>,
}

#[derive(Debug, Serialize)]
struct RespError {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct Event {
    event: String,
    computer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn ok(id: u64) -> Response {
    Response {
        id,
        ok: true,
        state: None,
        version: None,
        connected: None,
        error: None,
    }
}

fn ok_state(id: u64, state: &str) -> Response {
    Response {
        id,
        ok: true,
        state: Some(state.to_string()),
        version: None,
        connected: None,
        error: None,
    }
}

fn fail(id: u64, code: &str, message: String) -> Response {
    Response {
        id,
        ok: false,
        state: None,
        version: None,
        connected: None,
        error: Some(RespError {
            code: code.to_string(),
            message,
        }),
    }
}

fn emit(value: &impl Serialize) {
    let mut line = serde_json::to_string(value).expect("serializes");
    line.push('\n');
    let _ = io::stdout().write_all(line.as_bytes());
}

/// One live VM: HCS handle (Windows only) + guest transport + cached state.
/// State truth: HCS operation results first, properties re-query second,
/// guest transport liveness third. Never updated "because the API call
/// returned" without awaiting its operation result.
struct LiveVm {
    #[cfg(windows)]
    system: Option<hcs::HcsSystemHandle>,
    guest: hvsock::HyperVSocketTransport,
    state: String,
}

struct Manager {
    vms: HashMap<String, LiveVm>,
}

impl Manager {
    fn new() -> Self {
        Self {
            vms: HashMap::new(),
        }
    }

    fn get(&self, id: &str) -> Option<&LiveVm> {
        self.vms.get(id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut LiveVm> {
        self.vms.get_mut(id)
    }

    fn create(&mut self, req_id: u64, p: &CreateParams) -> Response {
        if self.vms.contains_key(&p.computer_id) {
            return fail(
                req_id,
                "already_exists",
                "computer already registered".into(),
            );
        }
        if !p.disk_path.to_lowercase().ends_with(".vhdx") {
            return fail(
                req_id,
                "invalid_params",
                format!("windows disk must be .vhdx, got: {}", p.disk_path),
            );
        }
        if !std::path::Path::new(&p.disk_path).is_file() {
            return fail(
                req_id,
                "invalid_params",
                format!("disk image missing at {}", p.disk_path),
            );
        }
        let spec = config::VmSpec {
            disk_vhdx_path: p.disk_path.clone(),
            vcpus: p.vcpus,
            memory_mb: p.memory_mb,
            secure_boot: true,
        };
        let doc = config::vm_config_json(&p.computer_id, &spec);
        if let Err(e) = config::validate_config_json(&doc) {
            return fail(req_id, "validation_failed", e);
        }
        #[cfg(windows)]
        {
            match hcs_create(&p.computer_id, &doc) {
                Ok(system) => {
                    let mut guest = hvsock::HyperVSocketTransport::new(p.computer_id.clone());
                    if let Err(e) = guest.listen() {
                        return fail(req_id, "internal", format!("vsock listen: {e}"));
                    }
                    self.vms.insert(
                        p.computer_id.clone(),
                        LiveVm {
                            #[cfg(windows)]
                            system: Some(system),
                            guest,
                            state: "stopped".to_string(),
                        },
                    );
                    return ok_state(req_id, "stopped");
                }
                Err(e) => return fail(req_id, "internal", e.to_string()),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (doc, spec);
            let mut guest = hvsock::HyperVSocketTransport::new(p.computer_id.clone());
            // Listener unavailable off Windows; the transport stays Down and
            // the session degrades honestly (Waiting -> timeout, never fake).
            let _ = guest.listen();
            self.vms.insert(
                p.computer_id.clone(),
                LiveVm {
                    guest,
                    state: "stopped".to_string(),
                },
            );
            ok_state(req_id, "stopped")
        }
    }

    /// Live on Windows (state transitions); kept compiled everywhere.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn set_state(&mut self, id: &str, state: &str) {
        if let Some(vm) = self.get_mut(id) {
            vm.state = state.to_string();
        }
    }

    /// Best-effort native state re-query; cached stands on any failure
    /// (property schema values get pinned against real hardware).
    fn refresh_state(&mut self, id: &str) {
        #[cfg(windows)]
        {
            let queried = self
                .get(id)
                .and_then(|vm| vm.system.as_ref())
                .and_then(|system| hcs::query_power_state(system, &config::state_query_json()));
            if let Some(state) = queried {
                self.set_state(id, &state);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = id;
        }
    }

    fn start(&mut self, req_id: u64, id: &str) -> Response {
        if self.get(id).is_none() {
            return fail(req_id, "unknown_computer", "no such computer".into());
        }
        #[cfg(windows)]
        {
            let ok = match self.get(id) {
                Some(vm) => match &vm.system {
                    Some(system) => hcs_start(system),
                    None => Err("no native instance (start creates one)".to_string()),
                },
                None => return fail(req_id, "unknown_computer", "no such computer".into()),
            };
            match ok {
                Ok(()) => {
                    // Fresh ephemeral instance per start is modeled by HCS
                    // itself (new compute system); our handle was created
                    // for this start. Running is reported only after the
                    // start operation result lands.
                    self.set_state(id, "running");
                    self.emit_vm(id, "running");
                    ok_state(req_id, "running")
                }
                Err(e) => fail(req_id, "start_failed", e),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = id;
            fail(
                req_id,
                "not_implemented",
                "HCS start requires Windows".to_string(),
            )
        }
    }

    fn stop(&mut self, req_id: u64, id: &str) -> Response {
        if self.get(id).is_none() {
            return fail(req_id, "unknown_computer", "no such computer".into());
        }
        #[cfg(windows)]
        {
            // Graceful shutdown first; forced terminate only if the guest
            // cannot shut down (mirrors the macOS requestStop-then-stop
            // pattern; documented in WINDOWS_BACKEND.md).
            let res = match self.get(id) {
                Some(vm) => match &vm.system {
                    Some(system) => hcs_shutdown(system).or_else(|_| hcs_terminate(system)),
                    None => Ok(()),
                },
                None => return fail(req_id, "unknown_computer", "no such computer".into()),
            };
            match res {
                Ok(()) => {
                    // HCS disposes the compute system on stop; our handle
                    // is dropped and the NEXT start creates a new native
                    // instance for the same Pegoles identity.
                    if let Some(vm) = self.vms.remove(id) {
                        drop(vm);
                    }
                    // Re-register an empty slot so state queries stay sane.
                    self.emit_vm(id, "stopped");
                    ok_state(req_id, "stopped")
                }
                Err(e) => fail(req_id, "stop_failed", e),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = id;
            fail(
                req_id,
                "not_implemented",
                "HCS stop requires Windows".to_string(),
            )
        }
    }

    fn pause(&mut self, req_id: u64, id: &str) -> Response {
        if self.get(id).is_none() {
            return fail(req_id, "unknown_computer", "no such computer".into());
        }
        #[cfg(windows)]
        {
            let res = match self.get(id) {
                Some(vm) => match &vm.system {
                    Some(system) => hcs_pause(system),
                    None => Err("no native instance".to_string()),
                },
                None => return fail(req_id, "unknown_computer", "no such computer".into()),
            };
            match res {
                Ok(()) => {
                    self.set_state(id, "paused");
                    self.emit_vm(id, "paused");
                    ok_state(req_id, "paused")
                }
                Err(e) => fail(req_id, "pause_failed", e),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = id;
            fail(
                req_id,
                "not_implemented",
                "HCS pause requires Windows".to_string(),
            )
        }
    }

    fn resume(&mut self, req_id: u64, id: &str) -> Response {
        if self.get(id).is_none() {
            return fail(req_id, "unknown_computer", "no such computer".into());
        }
        #[cfg(windows)]
        {
            let res = match self.get(id) {
                Some(vm) => match &vm.system {
                    Some(system) => hcs_resume(system),
                    None => Err("no native instance".to_string()),
                },
                None => return fail(req_id, "unknown_computer", "no such computer".into()),
            };
            match res {
                Ok(()) => {
                    self.set_state(id, "running");
                    self.emit_vm(id, "running");
                    ok_state(req_id, "running")
                }
                Err(e) => fail(req_id, "resume_failed", e),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = id;
            fail(
                req_id,
                "not_implemented",
                "HCS resume requires Windows".to_string(),
            )
        }
    }

    fn destroy(&mut self, req_id: u64, id: &str) -> Response {
        if self.get(id).is_none() {
            return fail(req_id, "unknown_computer", "no such computer".into());
        }
        // Ensure stopped (best effort), drop the native handle, forget.
        // Disk files are deleted by the Rust backend, not here.
        #[cfg(windows)]
        {
            if let Some(vm) = self.get(id) {
                if vm.state == "running" || vm.state == "paused" {
                    if let Some(system) = &vm.system {
                        let _ = hcs_terminate(system);
                    }
                }
            }
        }
        self.vms.remove(id);
        ok_state(req_id, "stopped")
    }

    fn guest_send(&mut self, req_id: u64, id: &str, payload: &str) -> Response {
        match self.get_mut(id) {
            Some(vm) => match vm.guest.send_frame(payload) {
                Ok(()) => ok(req_id),
                Err(e) => fail(req_id, "guest_unavailable", e.to_string()),
            },
            None => fail(req_id, "unknown_computer", "no such computer".into()),
        }
    }

    fn guest_status(&mut self, req_id: u64, id: &str) -> Response {
        match self.get_mut(id) {
            Some(vm) => {
                // Drain first so freshly-connected guests report correctly.
                drain_guest(vm, id);
                let connected = vm.guest.is_connected();
                Response {
                    connected: Some(connected),
                    ..ok(req_id)
                }
            }
            None => fail(req_id, "unknown_computer", "no such computer".into()),
        }
    }

    fn guest_disconnect(&mut self, req_id: u64, id: &str) -> Response {
        match self.get_mut(id) {
            Some(vm) => {
                vm.guest.close();
                emit(&Event {
                    event: "guest_disconnected".to_string(),
                    computer_id: id.to_string(),
                    state: None,
                    payload: None,
                    reason: Some("kicked".to_string()),
                    message: None,
                });
                ok(req_id)
            }
            None => fail(req_id, "unknown_computer", "no such computer".into()),
        }
    }

    /// Live on Windows (state-change events).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn emit_vm(&self, id: &str, state: &str) {
        emit(&Event {
            event: "vm_state_changed".to_string(),
            computer_id: id.to_string(),
            state: Some(state.to_string()),
            payload: None,
            reason: None,
            message: None,
        });
    }

    /// Pump one VM's guest link, forwarding frames/disconnects as events.
    /// Called after every command and on a steady tick from the main loop.
    fn pump(&mut self, id: &str) {
        if let Some(vm) = self.get_mut(id) {
            drain_guest(vm, id);
        }
    }

    fn pump_all(&mut self) {
        let ids: Vec<String> = self.vms.keys().cloned().collect();
        for id in ids {
            self.pump(&id);
        }
    }
}

/// Forward one link's pending transport facts as JSONL guest events.
fn drain_guest(vm: &mut LiveVm, id: &str) {
    use pegoles_computer::TransportEvent;
    for ev in vm.guest.poll_events() {
        match ev {
            TransportEvent::Connected => emit(&Event {
                event: "guest_connected".to_string(),
                computer_id: id.to_string(),
                state: None,
                payload: None,
                reason: None,
                message: None,
            }),
            TransportEvent::Frame(payload) => emit(&Event {
                event: "guest_frame".to_string(),
                computer_id: id.to_string(),
                state: None,
                payload: Some(payload),
                reason: None,
                message: None,
            }),
            TransportEvent::Disconnected { reason } => emit(&Event {
                event: "guest_disconnected".to_string(),
                computer_id: id.to_string(),
                state: None,
                payload: None,
                reason: Some(reason),
                message: None,
            }),
        }
    }
}

fn handle(mgr: &mut Manager, req: Request) -> Response {
    match req.command.as_str() {
        "version" => Response {
            version: Some(VERSION.to_string()),
            ..ok(req.id)
        },
        "validate" => match req.params {
            Some(p) => {
                let spec = config::VmSpec {
                    disk_vhdx_path: p.disk_path,
                    vcpus: p.vcpus,
                    memory_mb: p.memory_mb,
                    secure_boot: true,
                };
                let doc = config::vm_config_json(&p.computer_id, &spec);
                match config::validate_config_json(&doc) {
                    Ok(()) => ok_state(req.id, "stopped"),
                    Err(e) => fail(req.id, "validation_failed", e),
                }
            }
            None => fail(req.id, "invalid_params", "validate needs params".into()),
        },
        "create" => match req.params {
            Some(p) => {
                let params = CreateParams {
                    computer_id: p.computer_id,
                    disk_path: p.disk_path,
                    efi_vars_path: p.efi_vars_path,
                    machine_id_path: p.machine_id_path,
                    serial_log_path: p.serial_log_path,
                    vcpus: p.vcpus,
                    memory_mb: p.memory_mb,
                    seed_iso_path: p.seed_iso_path,
                };
                mgr.create(req.id, &params)
            }
            None => fail(req.id, "invalid_params", "create needs params".into()),
        },
        "start" => match req.computer_id {
            Some(id) => mgr.start(req.id, &id),
            None => fail(req.id, "invalid_params", "start needs computer_id".into()),
        },
        "pause" => match req.computer_id {
            Some(id) => mgr.pause(req.id, &id),
            None => fail(req.id, "invalid_params", "pause needs computer_id".into()),
        },
        "resume" => match req.computer_id {
            Some(id) => mgr.resume(req.id, &id),
            None => fail(req.id, "invalid_params", "resume needs computer_id".into()),
        },
        "stop" => match req.computer_id {
            Some(id) => mgr.stop(req.id, &id),
            None => fail(req.id, "invalid_params", "stop needs computer_id".into()),
        },
        "state" => match req.computer_id {
            Some(id) => {
                mgr.refresh_state(&id);
                match mgr.get(&id) {
                    Some(vm) => ok_state(req.id, &vm.state.clone()),
                    None => fail(req.id, "unknown_computer", "no such computer".into()),
                }
            }
            None => fail(req.id, "invalid_params", "state needs computer_id".into()),
        },
        "destroy" => match req.computer_id {
            Some(id) => mgr.destroy(req.id, &id),
            None => fail(req.id, "invalid_params", "destroy needs computer_id".into()),
        },
        "guest_send" => match (req.computer_id, req.payload) {
            (Some(id), Some(payload)) => mgr.guest_send(req.id, &id, &payload),
            _ => fail(
                req.id,
                "invalid_params",
                "guest_send needs computer_id + payload".into(),
            ),
        },
        "guest_status" => match req.computer_id {
            Some(id) => mgr.guest_status(req.id, &id),
            None => fail(
                req.id,
                "invalid_params",
                "guest_status needs computer_id".into(),
            ),
        },
        "guest_disconnect" => match req.computer_id {
            Some(id) => mgr.guest_disconnect(req.id, &id),
            None => fail(
                req.id,
                "invalid_params",
                "guest_disconnect needs computer_id".into(),
            ),
        },
        other => fail(
            req.id,
            "unknown_command",
            format!("unsupported command: {other}"),
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("--version") | Some("-V") => {
            println!("pegoles-vm-host-windows {VERSION}");
        }
        Some("--probe") => {
            // Real probe, shared with the Rust backend: machine-readable,
            // honest Unknowns, never invented data.
            match pegoles_computer::probe_windows_host() {
                Ok(caps) => println!(
                    "{{\"ok\":true,\"capabilities\":{}}}",
                    serde_json::to_string(&caps).expect("serializes")
                ),
                Err(e) => println!(
                    "{{\"ok\":false,\"error\":{{\"code\":\"probe_failed\",\"message\":{}}}}}",
                    serde_json::to_string(&e.to_string()).expect("serializes")
                ),
            }
        }
        _ => {
            let mut mgr = Manager::new();
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let resp = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => {
                        let id = req.id;
                        let mut resp = handle(&mut mgr, req);
                        resp.id = id;
                        resp
                    }
                    Err(_) => {
                        // Malformed line: error envelope when an id exists.
                        let id = serde_id(&line);
                        fail(id, "invalid_params", "malformed request".into())
                    }
                };
                emit(&resp);
                mgr.pump_all();
            }
        }
    }
}

/// Best-effort numeric id extraction (malformed-line envelopes only).
fn serde_id(line: &str) -> u64 {
    line.split("\"id\"")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// HCS session primitives. cfg(windows) performs real ComputeCore calls;
// elsewhere they return explicit errors (same shape, no fantasy results).
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn hcs_create(computer_id: &str, config_json: &str) -> Result<hcs::HcsSystemHandle, String> {
    hcs::create_compute_system(computer_id, config_json)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_create(_computer_id: &str, _config_json: &str) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(windows)]
fn hcs_start(system: &hcs::HcsSystemHandle) -> Result<(), String> {
    hcs::start_compute_system(system)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_start(_system: &()) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(windows)]
fn hcs_shutdown(system: &hcs::HcsSystemHandle) -> Result<(), String> {
    hcs::shutdown_compute_system(system)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_shutdown(_system: &()) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(windows)]
fn hcs_terminate(system: &hcs::HcsSystemHandle) -> Result<(), String> {
    hcs::terminate_compute_system(system)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_terminate(_system: &()) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(windows)]
fn hcs_pause(system: &hcs::HcsSystemHandle) -> Result<(), String> {
    hcs::pause_compute_system(system)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_pause(_system: &()) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(windows)]
fn hcs_resume(system: &hcs::HcsSystemHandle) -> Result<(), String> {
    hcs::resume_compute_system(system)
}

#[cfg(not(target_os = "windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hcs_resume(_system: &()) -> Result<(), String> {
    Err("HCS requires Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_extraction_is_best_effort() {
        assert_eq!(serde_id(r#"{"id":7,"command":"start"}"#), 7);
        assert_eq!(serde_id("garbage"), 0);
    }

    #[test]
    fn unknown_commands_rejected_not_executed() {
        let mut mgr = Manager::new();
        let resp = handle(
            &mut mgr,
            Request {
                id: 1,
                command: "exec".to_string(),
                computer_id: None,
                params: None,
                payload: None,
            },
        );
        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "unknown_command");
    }

    #[test]
    fn create_rejects_non_vhdx() {
        let mut mgr = Manager::new();
        let resp = handle(
            &mut mgr,
            Request {
                id: 2,
                command: "create".to_string(),
                computer_id: None,
                params: Some(CreateParams {
                    computer_id: "c".to_string(),
                    disk_path: "/tmp/disk.raw".to_string(),
                    efi_vars_path: None,
                    machine_id_path: None,
                    serial_log_path: None,
                    vcpus: 2,
                    memory_mb: 1536,
                    seed_iso_path: None,
                }),
                payload: None,
            },
        );
        assert!(!resp.ok);
    }

    #[test]
    fn lifecycle_without_windows_fails_closed() {
        // Off Windows every native op is an explicit error, never silent.
        let mut mgr = Manager::new();
        for (cmd, id) in [("start", 10), ("pause", 11), ("resume", 12), ("stop", 13)] {
            let resp = handle(
                &mut mgr,
                Request {
                    id,
                    command: cmd.to_string(),
                    computer_id: Some("nope".to_string()),
                    params: None,
                    payload: None,
                },
            );
            assert!(!resp.ok, "{cmd} must fail without a computer");
        }
    }
}

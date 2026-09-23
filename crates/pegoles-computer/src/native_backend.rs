//! Shared native-helper engine: child-process JSONL lifecycle + guest
//! session, identical on every host OS.
//!
//! `MacOSVirtualizationBackend` and `WindowsHcsBackend` are thin wrappers
//! supplying a [`BackendProfile`] (helper binary location, image policy,
//! disk naming, serial capture, capabilities, platform gate). Everything
//! else — request/response matching, event application, outbound queue,
//! handshake, heartbeat, ping/info round-trips — lives here exactly once.
//!
//! SECURITY: same invariants as before. The engine spawns exactly one
//! helper binary (resolved from explicit env or well-known paths) and
//! speaks the closed JSONL command set. No shell, no host execution.

use pegoles_protocol::{ComputerConfig, ComputerId, ComputerState, GuestRuntimeState};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crate::error::{ComputerError, Result};
use crate::guest::{
    GraphicalSessionInfo, GuestObservation, GuestSession, Outbound, SessionOutcome,
};
use crate::image::{ComputerImageManager, ImageSpec};
use crate::platform::BackendCapabilities;
use crate::traits::ComputerInstance;
use crate::vmhost_proto::{format_request, parse_line, CreateParams, HostCommand, HostLine};

pub const SHORT_TIMEOUT: Duration = Duration::from_secs(30);
pub const START_TIMEOUT: Duration = Duration::from_secs(150);
pub const STOP_TIMEOUT: Duration = Duration::from_secs(90);
/// Observations produced while a blocking round-trip was in flight are
/// kept (not dropped) until the next `poll_guest`. Bounded so a flooding
/// guest cannot grow host memory while nobody polls; oldest drop first.
pub const MAX_DEFERRED_OBSERVATIONS: usize = 256;
/// Everything that differs between the macOS and Windows native backends.
/// All other behavior is shared byte-for-byte.
#[derive(Clone, Debug)]
pub struct BackendProfile {
    /// Display name for diagnostics (e.g. "pegoles-vm-host").
    pub helper_display_name: &'static str,
    /// Env override for the helper binary location.
    pub helper_env_var: &'static str,
    /// Dev-tree candidates, relative to the pegoles-computer crate dir.
    pub dev_helper_relpaths: &'static [&'static str],
    /// File name inside the Tauri bundle `Resources/` dir.
    pub resource_helper_name: &'static str,
    /// Official image fallback spec (used only when no derived image and
    /// `allow_official_fallback` is set; e.g. macOS nocloud for dev).
    pub official_spec: ImageSpec,
    /// Whether create() may fall back to the official image. Windows sets
    /// false: a RAW fallback could never boot on Hyper-V, so missing
    /// derived VHDX must fail closed as ImageMissing.
    pub allow_official_fallback: bool,
    /// Instance disk file name (`disk.img` / `disk.vhdx`).
    pub disk_file_name: &'static str,
    /// Disk format of the instance disk (must match the file name).
    pub disk_format: crate::platform::DiskFormat,
    /// Capture the guest serial console to logs/serial.log. macOS yes
    /// (virtio console file attachment); Windows no (HCS COM capture is
    /// future work — the concept stays, the attachment differs).
    pub want_serial_log: bool,
    /// What this backend can actually do right now.
    pub capabilities: BackendCapabilities,
    /// Platform gate run by `new()` (never a silent Mock fallback).
    pub check_platform: fn() -> Result<()>,
}

/// Layout owned by Rust for one computer:
/// `<computers_dir>/<computer-id>/{metadata.json,disk.*,efi-vars.bin,
/// machine-id,logs/serial.log?}`. EFI/machine-id files are written by the
/// native helper on macOS; on Windows only metadata+disk+logs exist.
#[derive(Clone, Debug)]
pub struct ComputerPaths {
    pub dir: PathBuf,
    pub metadata: PathBuf,
    pub disk: PathBuf,
    pub efi_vars: PathBuf,
    pub machine_id: PathBuf,
    pub serial_log: Option<PathBuf>,
}

impl ComputerPaths {
    pub fn new(computers_dir: &Path, id: &ComputerId, profile: &BackendProfile) -> Self {
        let dir = computers_dir.join(id.to_string());
        let logs = dir.join("logs");
        Self {
            metadata: dir.join("metadata.json"),
            disk: dir.join(profile.disk_file_name),
            efi_vars: dir.join("efi-vars.bin"),
            machine_id: dir.join("machine-id"),
            serial_log: profile.want_serial_log.then(|| logs.join("serial.log")),
            dir,
        }
    }

    pub fn create_dirs(&self) -> Result<()> {
        if let Some(serial) = &self.serial_log {
            std::fs::create_dir_all(serial.parent().expect("logs dir"))
                .map_err(|e| ComputerError::Backend(e.to_string()))?;
        } else {
            std::fs::create_dir_all(&self.dir)
                .map_err(|e| ComputerError::Backend(e.to_string()))?;
        }
        Ok(())
    }
}

/// Resolve the helper binary: explicit env -> dev build tree ->
/// Tauri bundle resources next to the executable.
pub fn resolve_helper_binary(profile: &BackendProfile) -> Result<PathBuf> {
    if let Ok(p) = std::env::var(profile.helper_env_var) {
        let path = PathBuf::from(&p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(ComputerError::Backend(format!(
            "{} points at missing file: {p}",
            profile.helper_env_var
        )));
    }
    let mut candidates = Vec::new();
    // Dev tree: <repo>/... relative to this crate's manifest dir.
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        for rel in profile.dev_helper_relpaths {
            candidates.push(PathBuf::from(&manifest).join(rel));
        }
    }
    // Tauri bundle: <App>.app/Contents/Resources/<name>
    if let Ok(exe) = std::env::current_exe() {
        candidates.push(
            exe.join("..")
                .join("..")
                .join("Resources")
                .join(profile.resource_helper_name),
        );
    }
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(ComputerError::Backend(format!(
        "{} binary not found (build the native helper or set {})",
        profile.helper_display_name, profile.helper_env_var
    )))
}

/// Builds a helper transport for a backend profile. Installed once per
/// process by an embedding host (macOS desktop: in-process VM host that
/// speaks the SAME JSONL protocol and closed command set, so the VM and
/// its framebuffer view share one process). Without a factory the engine
/// spawns the helper binary as a child process (tests, bench, builder).
pub type HostTransportFactory =
    dyn Fn(&BackendProfile) -> Result<Box<dyn HostTransport>> + Send + Sync;

static HOST_TRANSPORT_FACTORY: std::sync::OnceLock<Box<HostTransportFactory>> =
    std::sync::OnceLock::new();

/// Install the process-wide transport factory. May be called once;
/// later calls fail (no silent swapping of the VM host underneath Core).
pub fn install_host_transport_factory(factory: Box<HostTransportFactory>) -> Result<()> {
    HOST_TRANSPORT_FACTORY
        .set(factory)
        .map_err(|_| ComputerError::Backend("host transport factory already installed".into()))
}

/// Whether an embedding host installed a transport factory.
pub fn host_transport_factory_installed() -> bool {
    HOST_TRANSPORT_FACTORY.get().is_some()
}

/// Byte transport to the helper. Production = child stdio; tests = fake.
pub trait HostTransport: Send {
    fn send(&mut self, line: &str) -> Result<()>;
    fn recv(&mut self, timeout: Duration) -> Result<String>;
    /// Non-blocking drain for the guest pump. None = nothing pending.
    fn try_recv(&mut self) -> Option<Result<String>>;
    fn alive(&mut self) -> bool;
}

struct ChildTransport {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<std::result::Result<String, String>>,
}

impl ChildTransport {
    pub(crate) fn spawn(binary: &Path, display_name: &str) -> Result<Self> {
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ComputerError::Backend(format!("cannot spawn {display_name}: {e}")))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let payload = line.map_err(|e| e.to_string());
                if tx.send(payload).is_err() {
                    break;
                }
            }
            let _ = tx.send(Err("vm host stdout closed".to_string()));
        });
        Ok(Self { child, stdin, rx })
    }
}

impl HostTransport for ChildTransport {
    fn send(&mut self, line: &str) -> Result<()> {
        if !self.alive() {
            return Err(ComputerError::BackendDisconnected(
                "native vm host exited".to_string(),
            ));
        }
        writeln!(self.stdin, "{line}").map_err(|e| {
            ComputerError::BackendDisconnected(format!("cannot write to vm host: {e}"))
        })?;
        self.stdin.flush().map_err(|e| {
            ComputerError::BackendDisconnected(format!("cannot flush vm host stdin: {e}"))
        })?;
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<String> {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(line)) => Ok(line),
            Ok(Err(e)) => Err(ComputerError::BackendDisconnected(e)),
            Err(_) => Err(ComputerError::Backend(format!(
                "vm host response timeout after {}s",
                timeout.as_secs()
            ))),
        }
    }

    fn try_recv(&mut self) -> Option<Result<String>> {
        match self.rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                ComputerError::BackendDisconnected("vm host stdout closed".to_string()),
            )),
            Ok(Ok(line)) => Some(Ok(line)),
            Ok(Err(e)) => Some(Err(ComputerError::BackendDisconnected(e))),
        }
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ChildTransport {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Scripted in-process fake of the helper for unit tests (no hypervisor).
/// Behaves like the real protocol: canned states per command plus async
/// `vm_state_changed` events queued after transitions, plus a scripted
/// guest vsock link (connect/hello/ready/ping/info).
pub struct FakeTransport {
    pub sent: Vec<String>,
    pending: VecDeque<String>,
    states: std::collections::HashMap<String, String>,
    /// After this many sends, the transport starts failing (crash simulation).
    pub fail_after_sends: Option<usize>,
    guest_links: std::collections::HashSet<String>,
    /// Scripted guest answer to `get_graphical_session` (one JSONL guest
    /// frame). `None` = the guest ignores the request (older runtime).
    pub graphical_session_reply: Option<String>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self {
            sent: Vec::new(),
            pending: VecDeque::new(),
            states: std::collections::HashMap::new(),
            fail_after_sends: None,
            guest_links: std::collections::HashSet::new(),
            graphical_session_reply: None,
        }
    }

    pub(crate) fn respond(
        &mut self,
        id: u64,
        state: Option<&str>,
        error: Option<(&str, &str)>,
    ) -> String {
        match (state, error) {
            (Some(s), _) => format!(r#"{{"id":{id},"ok":true,"state":"{s}"}}"#),
            (_, Some((code, msg))) => {
                format!(r#"{{"id":{id},"ok":false,"error":{{"code":"{code}","message":"{msg}"}}}}"#)
            }
            _ => format!(r#"{{"id":{id},"ok":true}}"#),
        }
    }

    /// Test helper: simulate the guest opening the vsock channel.
    pub fn inject_guest_connected(&mut self, computer_id: &str) {
        self.guest_links.insert(computer_id.to_string());
        self.pending.push_back(
            serde_json::json!({"event":"guest_connected","computer_id":computer_id}).to_string(),
        );
    }

    /// Test helper: simulate one complete guest frame arriving.
    pub fn inject_guest_frame(&mut self, computer_id: &str, payload: &str) {
        self.pending.push_back(
            serde_json::json!({"event":"guest_frame","computer_id":computer_id,"payload":payload})
                .to_string(),
        );
    }

    /// Test helper: the VM changes state on its own (crash, guest-side
    /// shutdown): queues the async event and makes later `state` queries
    /// report the same native truth.
    pub fn inject_vm_state(&mut self, computer_id: &str, state: &str) {
        self.states
            .insert(computer_id.to_string(), state.to_string());
        self.pending.push_back(
            serde_json::json!({"event":"vm_state_changed","computer_id":computer_id,"state":state})
                .to_string(),
        );
    }

    /// Test helper: simulate the guest channel closing.
    pub fn inject_guest_disconnected(&mut self, computer_id: &str, reason: &str) {
        self.guest_links.remove(computer_id);
        self.pending.push_back(
            serde_json::json!({"event":"guest_disconnected","computer_id":computer_id,"reason":reason})
                .to_string(),
        );
    }
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HostTransport for FakeTransport {
    fn send(&mut self, line: &str) -> Result<()> {
        if let Some(limit) = self.fail_after_sends {
            if self.sent.len() >= limit {
                return Err(ComputerError::BackendDisconnected(
                    "fake host crashed".to_string(),
                ));
            }
        }
        self.sent.push(line.to_string());
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| ComputerError::Backend(e.to_string()))?;
        let id = v["id"].as_u64().unwrap_or(0);
        let cmd = v["command"].as_str().unwrap_or("").to_string();
        let cid = v["computer_id"].as_str().unwrap_or("").to_string();
        // Guest control plane: scripted guest answers so ping/info
        // round-trips work without a VM. The command response is queued
        // first, then the async guest frame (real ordering).
        if cmd == "guest_send" && self.guest_links.contains(&cid) {
            let payload = v["payload"].as_str().unwrap_or("").to_string();
            let reply = self.respond(id, None, None);
            self.pending.push_back(reply);
            if payload.contains("\"ping\"") {
                let nonce = payload
                    .split("\"nonce\"")
                    .nth(1)
                    .and_then(|s| {
                        s.split(|c: char| !c.is_ascii_digit())
                            .find(|s| !s.is_empty())
                    })
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                self.pending.push_back(
                    serde_json::json!({"event":"guest_frame","computer_id":cid,
                        "payload": format!(r#"{{"type":"pong","nonce":{nonce}}}"#)})
                    .to_string(),
                );
            } else if payload.contains("get_graphical_session") {
                if let Some(reply) = self.graphical_session_reply.clone() {
                    self.pending.push_back(
                        serde_json::json!({"event":"guest_frame","computer_id":cid,
                            "payload": reply})
                        .to_string(),
                    );
                }
            } else if payload.contains("get_system_info") {
                self.pending.push_back(
                    serde_json::json!({"event":"guest_frame","computer_id":cid,
                        "payload": r#"{"type":"system_info","os":"debian","os_version":"13","kernel":"fake","arch":"aarch64","hostname":"fake","runtime_version":"0.1.0","protocol_version":1}"#})
                    .to_string(),
                );
            }
            return Ok(());
        }
        let reply = match cmd.as_str() {
            "version" => self
                .respond(id, None, None)
                .replace("true}", r#"true,"version":"0.1.0"}"#),
            "validate" => self.respond(id, Some("stopped"), None),
            "create" => {
                self.states.insert(cid.clone(), "stopped".into());
                self.respond(id, Some("stopped"), None)
            }
            "start" => {
                self.states.insert(cid.clone(), "running".into());
                self.pending.push_back(
                    serde_json::json!({"event":"vm_state_changed","computer_id":cid,"state":"running"}).to_string(),
                );
                self.respond(id, Some("running"), None)
            }
            "pause" => {
                self.states.insert(cid.clone(), "paused".into());
                self.respond(id, Some("paused"), None)
            }
            "resume" => {
                self.states.insert(cid.clone(), "running".into());
                self.respond(id, Some("running"), None)
            }
            "stop" => {
                self.states.insert(cid.clone(), "stopped".into());
                self.respond(id, Some("stopped"), None)
            }
            "state" => {
                let s = self
                    .states
                    .get(&cid)
                    .cloned()
                    .unwrap_or_else(|| "stopped".into());
                self.respond(id, Some(&s), None)
            }
            "destroy" => {
                self.states.remove(&cid);
                self.respond(id, Some("stopped"), None)
            }
            "guest_send" => {
                if self.guest_links.contains(&cid) {
                    self.respond(id, None, None)
                } else {
                    self.respond(id, None, Some(("guest_unavailable", "no guest connection")))
                }
            }
            "guest_status" => {
                let connected = self.guest_links.contains(&cid);
                format!(r#"{{"id":{id},"ok":true,"connected":{connected}}}"#)
            }
            "guest_disconnect" => {
                self.guest_links.remove(&cid);
                self.respond(id, None, None)
            }
            _ => self.respond(id, None, Some(("unknown_command", "unsupported command"))),
        };
        self.pending.push_back(reply);
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Result<String> {
        self.pending
            .pop_front()
            .ok_or_else(|| ComputerError::Backend("fake host has no reply".to_string()))
    }

    fn try_recv(&mut self) -> Option<Result<String>> {
        self.pending.pop_front().map(Ok)
    }

    fn alive(&mut self) -> bool {
        match self.fail_after_sends {
            Some(limit) => self.sent.len() < limit,
            None => true,
        }
    }
}

/// Shared handle around [`FakeTransport`]: the test keeps one clone to
/// script guest frames while the backend owns the other. Test support
/// (like `FakeTransport` itself); never used by production code.
#[derive(Clone, Default)]
pub struct SharedFakeTransport {
    inner: std::sync::Arc<std::sync::Mutex<FakeTransport>>,
}

impl SharedFakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Poison-tolerant access to the scripted fake.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, FakeTransport> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl HostTransport for SharedFakeTransport {
    fn send(&mut self, line: &str) -> Result<()> {
        self.lock().send(line)
    }
    fn recv(&mut self, timeout: Duration) -> Result<String> {
        self.lock().recv(timeout)
    }
    fn try_recv(&mut self) -> Option<Result<String>> {
        self.lock().try_recv()
    }
    fn alive(&mut self) -> bool {
        self.lock().alive()
    }
}

pub(crate) struct NativeInner {
    profile: BackendProfile,
    id: Option<ComputerId>,
    cached: ComputerState,
    config: Option<ComputerConfig>,
    paths: Option<ComputerPaths>,
    transport: Option<Box<dyn HostTransport>>,
    next_id: u64,
    guest: GuestSession,
    /// Session-requested vsock actions awaiting a free round-trip.
    pending_outbound: VecDeque<Outbound>,
    /// Ephemeral execution handle; minted per start, cleared on stop.
    instance: Option<ComputerInstance>,
    /// Observations seen during blocking round-trips (`call`,
    /// `wait_guest_frame`), handed to Core on the next `poll_guest` so no
    /// readiness fact is lost to a concurrent command.
    deferred: VecDeque<GuestObservation>,
}

/// Shared engine behind `MacOSVirtualizationBackend` and
/// `WindowsHcsBackend`. Constructed only via those wrappers (profile +
/// platform gate live there); tests use `with_transport`.
pub struct NativeHelperBackend {
    images_dir: PathBuf,
    computers_dir: PathBuf,
    inner: std::sync::Mutex<NativeInner>,
}

impl std::fmt::Debug for NativeHelperBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeHelperBackend")
            .field("images_dir", &self.images_dir)
            .field("computers_dir", &self.computers_dir)
            .finish()
    }
}

impl NativeHelperBackend {
    fn fresh_inner(profile: BackendProfile) -> NativeInner {
        NativeInner {
            profile,
            id: None,
            cached: ComputerState::Stopped,
            config: None,
            paths: None,
            transport: None,
            next_id: 1,
            guest: GuestSession::new(),
            pending_outbound: VecDeque::new(),
            instance: None,
            deferred: VecDeque::new(),
        }
    }

    fn defer_observations(inner: &mut NativeInner, observations: Vec<GuestObservation>) {
        for o in observations {
            if inner.deferred.len() >= MAX_DEFERRED_OBSERVATIONS {
                inner.deferred.pop_front();
            }
            inner.deferred.push_back(o);
        }
    }

    pub fn with_profile(
        images_dir: PathBuf,
        computers_dir: PathBuf,
        profile: BackendProfile,
    ) -> Self {
        Self {
            images_dir,
            computers_dir,
            inner: std::sync::Mutex::new(Self::fresh_inner(profile)),
        }
    }

    /// Test injection: pre-opened transport instead of spawning a process.
    pub fn with_transport(
        images_dir: PathBuf,
        computers_dir: PathBuf,
        profile: BackendProfile,
        transport: Box<dyn HostTransport>,
    ) -> Self {
        let backend = Self::with_profile(images_dir, computers_dir, profile);
        backend.inner.lock().expect("inner").transport = Some(transport);
        backend
    }

    pub fn serial_log_path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .expect("inner")
            .paths
            .clone()
            .and_then(|p| p.serial_log)
    }

    fn ensure_transport(inner: &mut NativeInner, binary_hint: Option<&Path>) -> Result<()> {
        if inner.transport.is_some() {
            return Ok(());
        }
        let profile = inner.profile.clone();
        if binary_hint.is_none() {
            if let Some(factory) = HOST_TRANSPORT_FACTORY.get() {
                inner.transport = Some(factory(&profile)?);
                return Ok(());
            }
        }
        let binary = match binary_hint {
            Some(p) => p.to_path_buf(),
            None => resolve_helper_binary(&profile)?,
        };
        inner.transport = Some(Box::new(ChildTransport::spawn(
            &binary,
            profile.helper_display_name,
        )?));
        Ok(())
    }

    /// Send a command and wait for the matching response id, applying any
    /// async events observed meanwhile. Disconnect forces cached Error.
    fn call(
        inner: &mut NativeInner,
        command: HostCommand,
        timeout: Duration,
    ) -> Result<Option<crate::vmhost_proto::VmState>> {
        if inner.transport.as_mut().map(|t| t.alive()) != Some(true) {
            inner.cached = ComputerState::Error;
            return Err(ComputerError::BackendDisconnected(format!(
                "{} is not running",
                inner.profile.helper_display_name
            )));
        }
        let id = inner.next_id;
        inner.next_id += 1;
        let line = format_request(id, &command);
        {
            let transport = inner.transport.as_mut().expect("transport");
            if let Err(e) = transport.send(&line) {
                inner.cached = ComputerState::Error;
                return Err(e);
            }
        }
        loop {
            let raw = {
                let transport = inner.transport.as_mut().expect("transport");
                match transport.recv(timeout) {
                    Ok(l) => l,
                    Err(e) => {
                        inner.cached = ComputerState::Error;
                        return Err(e);
                    }
                }
            };
            match parse_line(&raw) {
                Ok(HostLine::Response(resp)) if resp.id == id => {
                    if !resp.ok {
                        let err = resp
                            .error
                            .unwrap_or_else(|| crate::vmhost_proto::HostError {
                                code: crate::vmhost_proto::HostErrorCode::Internal,
                                message: String::new(),
                            });
                        Self::flush_outbound(inner, &computer_id_for(&command));
                        return match err.code {
                            crate::vmhost_proto::HostErrorCode::GuestUnavailable => {
                                Err(ComputerError::GuestUnavailable(err.message))
                            }
                            _ => Err(ComputerError::Backend(err.message)),
                        };
                    }
                    let state = resp.state;
                    Self::flush_outbound(inner, &computer_id_for(&command));
                    return Ok(state);
                }
                Ok(HostLine::Response(_)) => continue, // not ours; keep waiting
                Ok(HostLine::Event(ev)) => {
                    let mut obs = Vec::new();
                    Self::apply_event(inner, &computer_id_for(&command), ev, &mut obs);
                    Self::defer_observations(inner, obs);
                }
                Err(e) => {
                    return Err(ComputerError::Backend(format!("bad vm host line: {e}")));
                }
            }
        }
    }

    /// Feed one helper event to the VM cache + guest session. Outbound
    /// transport actions queue up; they flush after the current round-trip
    /// so nested calls can never swallow a foreign response id.
    fn apply_event(
        inner: &mut NativeInner,
        _computer_id: &Option<String>,
        ev: crate::vmhost_proto::HostEvent,
        observations: &mut Vec<GuestObservation>,
    ) {
        use crate::vmhost_proto::HostEvent as E;
        let now = std::time::Instant::now();
        let outcomes: Vec<SessionOutcome> = match ev {
            E::VmStateChanged { state, .. } => {
                inner.cached = state.as_protocol();
                Vec::new()
            }
            E::VmFailed { .. } => {
                inner.cached = ComputerState::Error;
                Vec::new()
            }
            E::GuestConnected { .. } => inner.guest.on_connected(now),
            E::GuestFrame { payload, .. } => inner.guest.on_frame(&payload, now),
            E::GuestDisconnected { reason, .. } => inner.guest.on_disconnected(reason),
        };
        for o in outcomes {
            let (action, observation) = o.split();
            if let Some(action) = action {
                inner.pending_outbound.push_back(action);
            }
            if let Some(o) = observation {
                observations.push(o);
            }
        }
    }

    /// Execute queued outbound guest actions (each its own round-trip).
    /// Bounded: a misbehaving peer cannot spin us forever.
    fn flush_outbound(inner: &mut NativeInner, computer_id: &Option<String>) {
        let Some(cid) = computer_id.clone() else {
            inner.pending_outbound.clear();
            return;
        };
        for _ in 0..32 {
            let action = match inner.pending_outbound.pop_front() {
                Some(a) => a,
                None => break,
            };
            match action {
                Outbound::Send(frame) => {
                    if Self::guest_send_frame(inner, &cid, &frame).is_err() {
                        for o in inner.guest.on_disconnected("send_failed".to_string()) {
                            let (a, _) = o.split();
                            if let Some(a) = a {
                                inner.pending_outbound.push_back(a);
                            }
                        }
                    }
                }
                Outbound::Kick => Self::guest_kick(inner, &cid),
            }
        }
    }

    fn op(&self, command: HostCommand, timeout: Duration) -> Result<ComputerState> {
        let mut inner = self.inner.lock().expect("inner");
        Self::ensure_transport(&mut inner, None)?;
        let state = Self::call(&mut inner, command, timeout)?.map(|s| s.as_protocol());
        if let Some(s) = state {
            inner.cached = s;
        }
        Ok(inner.cached)
    }

    /// Send one guest frame; maps helper errors to typed variants.
    fn guest_send_frame(inner: &mut NativeInner, computer_id: &str, payload: &str) -> Result<()> {
        Self::call(
            inner,
            HostCommand::GuestSend {
                computer_id: computer_id.to_string(),
                payload: payload.to_string(),
            },
            SHORT_TIMEOUT,
        )?;
        Ok(())
    }

    /// Drop the guest connection; best effort (never fails the pump).
    fn guest_kick(inner: &mut NativeInner, computer_id: &str) {
        let _ = Self::call(
            inner,
            HostCommand::GuestDisconnect {
                computer_id: computer_id.to_string(),
            },
            SHORT_TIMEOUT,
        );
    }

    /// Non-blocking guest pump: drain transport lines, flush queued
    /// outbound frames, run session tick (handshake timeout, heartbeat).
    fn pump_guest_inner(inner: &mut NativeInner) -> Vec<GuestObservation> {
        let mut observations: Vec<GuestObservation> = inner.deferred.drain(..).collect();
        let computer_id = inner.id.map(|id| id.to_string());
        if inner.transport.as_mut().map(|t| t.alive()) != Some(true) {
            return observations;
        }
        loop {
            let line = {
                let transport = inner.transport.as_mut().expect("transport");
                match transport.try_recv() {
                    None => break,
                    Some(Err(_)) => {
                        inner.cached = ComputerState::Error;
                        Self::apply_event(
                            inner,
                            &None,
                            crate::vmhost_proto::HostEvent::GuestDisconnected {
                                computer_id: String::new(),
                                reason: "helper_gone".to_string(),
                            },
                            &mut observations,
                        );
                        break;
                    }
                    Some(Ok(l)) => l,
                }
            };
            match parse_line(&line) {
                Ok(HostLine::Response(_)) => {} // stray; calls consume their own
                Ok(HostLine::Event(ev)) => {
                    Self::apply_event(inner, &computer_id, ev, &mut observations)
                }
                Err(_) => {} // transport garbage during poll: ignore, stay live
            }
        }
        Self::flush_outbound(inner, &computer_id);
        for o in inner.guest.tick(std::time::Instant::now()) {
            let (action, observation) = o.split();
            if let Some(action) = action {
                inner.pending_outbound.push_back(action);
            }
            if let Some(o) = observation {
                observations.push(o);
            }
        }
        Self::flush_outbound(inner, &computer_id);
        observations
    }

    /// Blocking wait for one guest observation satisfying `accept`,
    /// feeding all other frames to the session. Used by ping/info
    /// round-trips. Transport death forces cached Error like `call()`.
    fn wait_guest_frame(
        inner: &mut NativeInner,
        computer_id: &str,
        timeout: Duration,
        mut accept: impl FnMut(&GuestObservation) -> bool,
    ) -> Result<()> {
        let deadline = std::time::Instant::now() + timeout;
        let cid = Some(computer_id.to_string());
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(ComputerError::Backend("guest response timeout".to_string()));
            }
            let remaining = (deadline - now).min(Duration::from_secs(5));
            let line = {
                let transport = inner.transport.as_mut().ok_or(ComputerError::NotCreated)?;
                match transport.recv(remaining) {
                    Ok(l) => l,
                    Err(e) => {
                        inner.cached = ComputerState::Error;
                        return Err(e);
                    }
                }
            };
            match parse_line(&line) {
                Ok(HostLine::Event(ev)) => {
                    let mut obs = Vec::new();
                    Self::apply_event(inner, &cid, ev, &mut obs);
                    let accepted = obs.iter().any(&mut accept);
                    // Every observation (accepted or not) still reaches
                    // Core on the next poll: nothing is swallowed.
                    Self::defer_observations(inner, obs);
                    Self::flush_outbound(inner, &cid);
                    if accepted {
                        return Ok(());
                    }
                }
                Ok(HostLine::Response(_)) => continue,
                Err(_) => continue,
            }
        }
    }

    // --- backend operations (thin wrappers call these) ---

    pub fn backend_create(&self, config: ComputerConfig) -> Result<ComputerId> {
        crate::config::validate_config(&config)?;
        let mut inner = self.inner.lock().expect("inner");
        if let Some(id) = inner.id {
            return Err(ComputerError::AlreadyCreated(id));
        }
        // Boot source: derived image when Ready, else the official image
        // for this profile's spec (macOS dev flow). Missing both fails
        // closed; Windows disables the official fallback (RAW could never
        // boot on Hyper-V).
        let profile = inner.profile.clone();
        // Build-time escape hatch: PEGOLES_IMAGE_SPEC=generic makes image
        // builders boot the cloud-init source image instead of the normal
        // official spec. Never set in the normal lifecycle.
        let spec = match std::env::var("PEGOLES_IMAGE_SPEC").as_deref() {
            Ok("generic") => crate::image::GENERIC_DEBIAN_13_ARM64,
            _ => profile.official_spec,
        };
        let images = ComputerImageManager::with_spec(self.images_dir.clone(), spec);
        let id = ComputerId::new();
        let paths = ComputerPaths::new(&self.computers_dir, &id, &profile);
        paths.create_dirs()?;
        images.instantiate_boot_source_for_format_with_fallback(
            &id,
            &paths.disk,
            profile.disk_format,
            profile.allow_official_fallback,
        )?;
        let metadata = serde_json::json!({
            "computer_id": id.to_string(),
            "vcpus": config.vcpus,
            "memory_mb": config.memory_mb,
            "disk_gb": config.disk_gb,
        });
        std::fs::write(
            &paths.metadata,
            serde_json::to_string_pretty(&metadata).expect("metadata serializes"),
        )
        .map_err(|e| ComputerError::Backend(e.to_string()))?;

        Self::ensure_transport(&mut inner, None)?;
        // Build-time provisioning only: PEGOLES_SEED_ISO attaches a
        // read-only cloud-init seed ISO as a second disk. Used solely by
        // image builders; never set in the normal lifecycle. (Read here,
        // not plumbed through the API, precisely so product code paths
        // cannot enable it by accident.)
        let seed_iso_path = std::env::var("PEGOLES_SEED_ISO").ok().filter(|p| {
            let ok = std::path::Path::new(p).is_file();
            if !ok {
                eprintln!("PEGOLES_SEED_ISO points at missing file: {p}");
            }
            ok
        });
        let params = CreateParams {
            computer_id: id.to_string(),
            disk_path: paths.disk.to_string_lossy().into_owned(),
            efi_vars_path: paths.efi_vars.to_string_lossy().into_owned(),
            machine_id_path: paths.machine_id.to_string_lossy().into_owned(),
            serial_log_path: paths
                .serial_log
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            vcpus: config.vcpus,
            memory_mb: config.memory_mb,
            seed_iso_path,
            display: config.display.map(|d| crate::vmhost_proto::DisplayParams {
                width_px: d.width_px,
                height_px: d.height_px,
            }),
        };
        let state = Self::call(&mut inner, HostCommand::Create { params }, SHORT_TIMEOUT)?
            .map(|s| s.as_protocol())
            .unwrap_or(ComputerState::Stopped);
        inner.id = Some(id);
        inner.cached = state;
        inner.config = Some(config);
        inner.paths = Some(paths);
        Ok(id)
    }

    pub fn backend_start(&self) -> Result<ComputerState> {
        let id = self.computer_id().ok_or(ComputerError::NotCreated)?;
        // Rotate the serial log while the VM is stopped (never truncate a
        // file the helper has open for append — that would sparse-hole it).
        // Cap 8 MiB, keep the last 1 MiB: boot logs are ~100 KiB, so this
        // is a backstop, not a routine event. No heartbeat noise is logged
        // anywhere by design.
        self.enforce_log_caps();
        let state = self.op(
            HostCommand::Start {
                computer_id: id.to_string(),
            },
            START_TIMEOUT,
        )?;
        // VM Running starts the guest-readiness clock (Waiting) and mints
        // a fresh ephemeral instance; the computer identity stays stable
        // across restarts (HCS parity).
        let mut inner = self.inner.lock().expect("inner");
        inner.guest.on_vm_started(std::time::Instant::now());
        inner.deferred.clear();
        inner.instance = Some(ComputerInstance::new(id));
        Ok(state)
    }

    pub fn backend_stop(&self) -> Result<ComputerState> {
        let id = self.computer_id().ok_or(ComputerError::NotCreated)?;
        let state = self.op(
            HostCommand::Stop {
                computer_id: id.to_string(),
            },
            STOP_TIMEOUT,
        )?;
        let mut inner = self.inner.lock().expect("inner");
        inner.guest.on_vm_stopped();
        inner.deferred.clear();
        inner.instance = None;
        drop(inner);
        self.enforce_log_caps();
        Ok(state)
    }

    /// Truncate `logs/serial.log` to its last megabyte when it exceeds
    /// 8 MiB. Call only while the VM is stopped (see backend_start).
    pub fn enforce_log_caps(&self) {
        const CAP: u64 = 8 * 1024 * 1024;
        const KEEP: u64 = 1024 * 1024;
        let Some(path) = self.serial_log_path() else {
            return;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        if meta.len() <= CAP {
            return;
        }
        if let Ok(content) = std::fs::read(&path) {
            let tail = &content[content.len().saturating_sub(KEEP as usize)..];
            // Cut at a UTF-8 boundary (at most 3 bytes in).
            let mut start = 0;
            while start < tail.len().min(4) && std::str::from_utf8(&tail[start..]).is_err() {
                start += 1;
            }
            let _ = std::fs::write(&path, &tail[start..]);
        }
    }

    pub fn backend_pause(&self) -> Result<ComputerState> {
        let id = self.computer_id().ok_or(ComputerError::NotCreated)?;
        self.op(
            HostCommand::Pause {
                computer_id: id.to_string(),
            },
            SHORT_TIMEOUT,
        )
    }

    pub fn backend_resume(&self) -> Result<ComputerState> {
        let id = self.computer_id().ok_or(ComputerError::NotCreated)?;
        self.op(
            HostCommand::Resume {
                computer_id: id.to_string(),
            },
            SHORT_TIMEOUT,
        )
    }

    pub fn backend_reset(&self) -> Result<ComputerState> {
        let current = self.inner.lock().expect("inner").cached;
        if matches!(
            current,
            ComputerState::Running | ComputerState::Paused | ComputerState::Error
        ) {
            self.backend_stop()?;
        } else {
            let mut inner = self.inner.lock().expect("inner");
            inner.guest.on_vm_stopped();
            inner.deferred.clear();
            inner.instance = None;
        }
        Ok(ComputerState::Stopped)
    }

    pub fn backend_destroy(&self) -> Result<()> {
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        // Best effort native destroy; local state clears regardless so a
        // half-dead helper cannot pin the computer forever.
        let _ = Self::call(
            &mut inner,
            HostCommand::Destroy {
                computer_id: id.clone(),
            },
            SHORT_TIMEOUT,
        );
        if let Some(paths) = inner.paths.clone() {
            let _ = std::fs::remove_dir_all(&paths.dir);
        }
        inner.id = None;
        inner.config = None;
        inner.paths = None;
        inner.cached = ComputerState::Stopped;
        inner.instance = None;
        inner.guest.on_vm_stopped();
        inner.pending_outbound.clear();
        inner.deferred.clear();
        Ok(())
    }

    pub fn computer_id(&self) -> Option<ComputerId> {
        self.inner.lock().expect("inner").id
    }

    pub fn backend_state(&self) -> ComputerState {
        let mut inner = self.inner.lock().expect("inner");
        if inner.id.is_none() {
            return ComputerState::Stopped;
        }
        // Native truth first; fall back to cache if the helper is quiet.
        if inner.transport.as_mut().map(|t| t.alive()) == Some(true) {
            let id = inner.id.expect("id").to_string();
            if let Ok(Some(s)) = Self::call(
                &mut inner,
                HostCommand::State { computer_id: id },
                SHORT_TIMEOUT,
            ) {
                inner.cached = s.as_protocol();
            }
        }
        inner.cached
    }

    pub fn backend_config(&self) -> Option<ComputerConfig> {
        self.inner.lock().expect("inner").config.clone()
    }

    pub fn backend_instance(&self) -> Option<ComputerInstance> {
        self.inner.lock().expect("inner").instance.clone()
    }

    pub fn backend_capabilities(&self) -> crate::platform::BackendCapabilities {
        self.inner
            .lock()
            .expect("inner")
            .profile
            .capabilities
            .clone()
    }

    pub fn guest_state(&self) -> GuestRuntimeState {
        self.inner.lock().expect("inner").guest.state()
    }

    pub fn guest_info(&self) -> Option<pegoles_guest_proto::SystemInfo> {
        self.inner.lock().expect("inner").guest.guest_info()
    }

    pub fn guest_ready_ms(&self) -> Option<u64> {
        self.inner.lock().expect("inner").guest.ready_duration_ms()
    }

    pub fn capability_diagnostics(&self) -> Vec<crate::CapabilityDiagnostic> {
        self.inner
            .lock()
            .expect("inner")
            .guest
            .capability_diagnostics()
            .to_vec()
    }

    pub fn graphical_session(&self) -> GraphicalSessionInfo {
        self.inner.lock().expect("inner").guest.graphical_session()
    }

    pub fn poll_guest(&self) -> Vec<GuestObservation> {
        let mut inner = self.inner.lock().expect("inner");
        if inner.id.is_none() {
            return Vec::new();
        }
        Self::pump_guest_inner(&mut inner)
    }

    /// `GuestTransport` facet pieces (used by the thin wrappers' trait
    /// impls). Same channel the session pump uses; never feeds the session
    /// itself — VM state events update the cache, guest events surface raw.
    pub fn transport_send_frame(&self, payload: &str) -> Result<()> {
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        Self::guest_send_frame(&mut inner, &id, payload)
    }

    pub fn transport_poll_events(&self) -> Vec<crate::transport::TransportEvent> {
        use crate::transport::TransportEvent as T;
        use crate::vmhost_proto::HostEvent as E;
        let mut inner = self.inner.lock().expect("inner");
        let Some(transport) = inner.transport.as_mut() else {
            return Vec::new();
        };
        if !transport.alive() {
            return Vec::new();
        }
        let mut out = Vec::new();
        loop {
            let line = {
                let transport = inner.transport.as_mut().expect("transport");
                match transport.try_recv() {
                    None => break,
                    Some(Err(_)) => {
                        inner.cached = ComputerState::Error;
                        out.push(T::Disconnected {
                            reason: "helper_gone".to_string(),
                        });
                        break;
                    }
                    Some(Ok(l)) => l,
                }
            };
            match parse_line(&line) {
                Ok(HostLine::Event(E::GuestConnected { .. })) => {
                    out.push(T::Connected);
                }
                Ok(HostLine::Event(E::GuestFrame { payload, .. })) => {
                    out.push(T::Frame(payload));
                }
                Ok(HostLine::Event(E::GuestDisconnected { reason, .. })) => {
                    out.push(T::Disconnected { reason });
                }
                Ok(HostLine::Event(E::VmStateChanged { state, .. })) => {
                    inner.cached = state.as_protocol();
                }
                Ok(HostLine::Event(E::VmFailed { .. })) => {
                    inner.cached = ComputerState::Error;
                }
                Ok(HostLine::Response(_)) => {} // owned by in-flight calls
                Err(_) => {}                    // stay live on garbage
            }
        }
        out
    }

    pub fn transport_close(&self) {
        let mut inner = self.inner.lock().expect("inner");
        let Some(id) = inner.id.map(|id| id.to_string()) else {
            return;
        };
        Self::guest_kick(&mut inner, &id);
    }

    pub fn transport_is_connected(&self) -> bool {
        self.inner.lock().expect("inner").guest.is_connected()
    }

    pub fn guest_ping(&self, timeout: Duration) -> Result<u64> {
        use pegoles_protocol::GuestRuntimeState as G;
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        if inner.guest.state() != G::Ready {
            return Err(ComputerError::GuestUnavailable(
                "guest runtime is not ready".to_string(),
            ));
        }
        let now = std::time::Instant::now();
        let (nonce, frame) = inner.guest.manual_ping(now).expect("ready checked");
        Self::guest_send_frame(&mut inner, &id, &frame)?;
        let mut latency = None;
        Self::wait_guest_frame(&mut inner, &id, timeout, |o| match o {
            GuestObservation::PongReceived {
                nonce: n,
                latency_ms,
            } if *n == nonce => {
                latency = Some(*latency_ms);
                true
            }
            _ => false,
        })?;
        latency.ok_or_else(|| ComputerError::Backend("pong lost".to_string()))
    }

    pub fn guest_info_request(&self, timeout: Duration) -> Result<pegoles_guest_proto::SystemInfo> {
        use pegoles_protocol::GuestRuntimeState as G;
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        if inner.guest.state() != G::Ready {
            return Err(ComputerError::GuestUnavailable(
                "guest runtime is not ready".to_string(),
            ));
        }
        let frame =
            pegoles_guest_proto::encode_host(&pegoles_guest_proto::HostMessage::GetSystemInfo);
        Self::guest_send_frame(&mut inner, &id, &frame)?;
        Self::wait_guest_frame(&mut inner, &id, timeout, |o| {
            matches!(o, GuestObservation::InfoReceived(_))
        })?;
        inner
            .guest
            .guest_info()
            .ok_or_else(|| ComputerError::Backend("system info lost".to_string()))
    }

    // --- agent input plane (Phase 5): guest channel only. No host input
    // API exists anywhere in this engine; every primitive round-trips the
    // guest runtime over vsock and waits for its acknowledgement.

    pub fn input_available(&self) -> bool {
        use pegoles_protocol::GuestRuntimeState as G;
        let inner = self.inner.lock().expect("inner");
        inner.guest.state() == G::Ready
            && inner
                .guest
                .guest_supports(pegoles_guest_proto::GUEST_CAP_INPUT)
    }

    pub fn input_capabilities(&self) -> crate::input::InputCapabilities {
        use crate::input::{InputBackendKind, InputCapabilities};
        let available = self.input_available();
        let frame = {
            let inner = self.inner.lock().expect("inner");
            use pegoles_protocol::GuestRuntimeState as G;
            inner.guest.state() == G::Ready
                && inner
                    .guest
                    .guest_supports(pegoles_guest_proto::GUEST_CAP_FRAME)
        };
        InputCapabilities {
            pointer: available,
            keyboard: available,
            screenshot: available && frame,
            max_text_len: pegoles_protocol::limits::MAX_TYPE_CHARS,
            kind: InputBackendKind::GuestChannel,
        }
    }

    /// Guest size accompanying input requests: session report wins,
    /// else the configured scanout (real VM config), else omitted.
    fn input_display_size(inner: &NativeInner) -> Option<pegoles_guest_proto::GuestDisplaySize> {
        None.or_else(|| {
            let g = inner.guest.graphical_session();
            match (g.width_px, g.height_px) {
                (Some(w), Some(h)) if w > 0 && h > 0 => {
                    Some(pegoles_guest_proto::GuestDisplaySize {
                        width_px: w,
                        height_px: h,
                    })
                }
                _ => inner.config.as_ref().and_then(|c| c.display).map(|d| {
                    pegoles_guest_proto::GuestDisplaySize {
                        width_px: d.width_px,
                        height_px: d.height_px,
                    }
                }),
            }
        })
    } // end input_display_size

    fn require_input_ready(inner: &crate::guest::GuestSession, capability: &str) -> Result<()> {
        use pegoles_protocol::GuestRuntimeState as G;
        if inner.state() != G::Ready {
            return Err(ComputerError::GuestUnavailable(
                "guest runtime is not ready".to_string(),
            ));
        }
        if !inner.guest_supports(capability) {
            return Err(ComputerError::UnsupportedOperation(
                inner.missing_capability_reason(capability),
            ));
        }
        Ok(())
    }

    pub fn input_execute(
        &self,
        request_id: &str,
        op: &crate::input::InputOp,
    ) -> crate::input::InputOutcome {
        let start = std::time::Instant::now();
        match self.input_execute_inner(request_id, op) {
            Ok(()) => crate::input::InputOutcome::ok(
                start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            ),
            Err(e) => crate::input::InputOutcome::failed(e.to_string()),
        }
    }

    fn input_execute_inner(&self, request_id: &str, op: &crate::input::InputOp) -> Result<()> {
        use crate::input::InputOp as Op;
        if request_id.len() > pegoles_guest_proto::MAX_REQUEST_ID_BYTES {
            return Err(ComputerError::Backend(
                "request id exceeds bound".to_string(),
            ));
        }
        // Observation ops travel their own paths (capture assembles
        // chunks; display info is answered from session facts).
        if matches!(op, Op::CaptureFrame { .. } | Op::GetDisplayInfo) {
            return Err(ComputerError::Backend(
                "wrong dispatch path for observation op".to_string(),
            ));
        }
        let guest_op = crate::input::op_to_guest(op)
            .ok_or_else(|| ComputerError::Backend("unmappable input op".to_string()))?;
        if !guest_op.is_bounded() {
            return Err(ComputerError::Backend(
                "input op exceeds guest bounds".to_string(),
            ));
        }
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        Self::require_input_ready(&inner.guest, pegoles_guest_proto::GUEST_CAP_INPUT)?;
        let frame = pegoles_guest_proto::encode_host(&pegoles_guest_proto::HostMessage::Input {
            request_id: request_id.to_string(),
            op: guest_op,
            display: Self::input_display_size(&inner),
        });
        Self::guest_send_frame(&mut inner, &id, &frame)?;
        let mut ack: Option<(bool, Option<String>)> = None;
        let wanted = request_id.to_string();
        Self::wait_guest_frame(
            &mut inner,
            &id,
            std::time::Duration::from_millis(pegoles_protocol::limits::INPUT_ROUNDTRIP_MS),
            |o| match o {
                GuestObservation::InputAckReceived {
                    request_id,
                    ok,
                    error,
                } if *request_id == wanted => {
                    ack = Some((*ok, error.clone()));
                    true
                }
                _ => false,
            },
        )?;
        match ack {
            Some((true, _)) => Ok(()),
            Some((false, error)) => Err(ComputerError::Backend(
                error.unwrap_or_else(|| "guest rejected input".to_string()),
            )),
            None => Err(ComputerError::Backend("input ack lost".to_string())),
        }
    }

    pub fn input_capture_frame(
        &self,
        request_id: &str,
        timeout: Duration,
    ) -> Result<crate::input::CapturedFrame> {
        let start = std::time::Instant::now();
        let (width_px, height_px, bytes) = self.input_capture_bytes(request_id, timeout)?;
        let byte_len = bytes.len() as u64;
        let computer_id = self
            .inner
            .lock()
            .expect("inner")
            .id
            .ok_or(ComputerError::NotCreated)?;
        Ok(crate::input::CapturedFrame {
            meta: pegoles_protocol::ObservedFrameMeta {
                frame_id: pegoles_protocol::FrameId::new(),
                computer_id,
                captured_at: chrono::Utc::now(),
                width_px,
                height_px,
                encoding: pegoles_protocol::FrameEncoding::RawRgba,
                byte_len,
                capture_latency_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            },
            bytes,
        })
    }

    fn input_capture_bytes(
        &self,
        request_id: &str,
        timeout: Duration,
    ) -> Result<(u32, u32, Vec<u8>)> {
        if request_id.len() > pegoles_guest_proto::MAX_REQUEST_ID_BYTES {
            return Err(ComputerError::Backend(
                "request id exceeds bound".to_string(),
            ));
        }
        let mut inner = self.inner.lock().expect("inner");
        let id = inner.id.ok_or(ComputerError::NotCreated)?.to_string();
        Self::require_input_ready(&inner.guest, pegoles_guest_proto::GUEST_CAP_FRAME)?;
        let frame = pegoles_guest_proto::encode_host(&pegoles_guest_proto::HostMessage::GetFrame {
            request_id: request_id.to_string(),
        });
        Self::guest_send_frame(&mut inner, &id, &frame)?;
        let deadline = std::time::Instant::now() + timeout;
        let wanted = request_id.to_string();
        // 1. FrameBegin carries dimensions + chunk count. An explicit
        // InputAck{ok:false} with the same id means the guest cannot
        // capture (fail fast instead of timing out a 30 s transfer).
        let mut begin: Option<(u32, u32, u32)> = None;
        let mut refused: Option<String> = None;
        Self::wait_guest_frame(&mut inner, &id, timeout, |o| match o {
            GuestObservation::FrameBeginReceived {
                request_id,
                width_px,
                height_px,
                total_chunks,
            } if *request_id == wanted => {
                begin = Some((*width_px, *height_px, *total_chunks));
                true
            }
            GuestObservation::InputAckReceived {
                request_id,
                ok: false,
                error,
            } if *request_id == wanted => {
                refused = Some(
                    error
                        .clone()
                        .unwrap_or_else(|| "guest refused capture".to_string()),
                );
                true
            }
            _ => false,
        })?;
        if let Some(reason) = refused {
            return Err(ComputerError::UnsupportedOperation(reason));
        }
        let (width_px, height_px, total) =
            begin.ok_or_else(|| ComputerError::Backend("frame begin lost".to_string()))?;
        if total > pegoles_guest_proto::MAX_FRAME_CHUNKS {
            return Err(ComputerError::Backend(
                "frame exceeds chunk cap".to_string(),
            ));
        }
        let expect = width_px as u64 * height_px as u64 * 4;
        if expect > pegoles_protocol::limits::MAX_FRAME_BYTES {
            return Err(ComputerError::Backend("frame exceeds byte cap".to_string()));
        }
        // 2. Collect every chunk (any order; defers preserve the rest).
        let mut parts: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        while (parts.len() as u32) < total {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(ComputerError::Backend("frame capture timeout".to_string()));
            }
            let remaining = deadline - now;
            let wanted = request_id.to_string();
            Self::wait_guest_frame(&mut inner, &id, remaining, |o| match o {
                GuestObservation::FrameChunkReceived {
                    request_id,
                    seq,
                    bytes,
                } if *request_id == wanted && !parts.contains_key(seq) => {
                    parts.insert(*seq, bytes.clone());
                    true
                }
                _ => false,
            })?;
        }
        let mut chunks = Vec::with_capacity(total as usize);
        for seq in 0..total {
            chunks.push(
                parts
                    .remove(&seq)
                    .ok_or_else(|| ComputerError::Backend("frame incomplete".to_string()))?,
            );
        }
        let bytes = crate::input::reassemble_chunks(&chunks)?;
        if bytes.len() as u64 != expect {
            return Err(ComputerError::Backend(format!(
                "frame stride mismatch: {} bytes for {width_px}x{height_px} RGBA",
                bytes.len()
            )));
        }
        Ok((width_px, height_px, bytes))
    }
}

/// Computer id carried by a helper command, if any (for event routing).
fn computer_id_for(command: &HostCommand) -> Option<String> {
    match command {
        HostCommand::Version => None,
        HostCommand::Validate { params } => Some(params.computer_id.clone()),
        HostCommand::Create { params } => Some(params.computer_id.clone()),
        HostCommand::Start { computer_id }
        | HostCommand::Pause { computer_id }
        | HostCommand::Resume { computer_id }
        | HostCommand::Stop { computer_id }
        | HostCommand::State { computer_id }
        | HostCommand::Destroy { computer_id }
        | HostCommand::GuestSend { computer_id, .. }
        | HostCommand::GuestStatus { computer_id }
        | HostCommand::GuestDisconnect { computer_id } => Some(computer_id.clone()),
    }
}

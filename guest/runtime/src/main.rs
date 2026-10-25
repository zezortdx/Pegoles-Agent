//! pegoles-guest-runtime v0.1 — guest side of the Core<->Guest control plane.
//!
//! Linux/aarch64 only for production. Responsibilities:
//! connect to the host over AF_VSOCK, handshake (GuestHello -> HostHello ->
//! Ready), answer Ping/GetSystemInfo, reconnect with capped backoff,
//! exit cleanly when systemd stops us (stateless reconnect makes SIGTERM safe).
//!
//! Deliberately absent: GUI, browser, LLM, database, HTTP server, shell,
//! filesystem access, process execution. See docs/GUEST_PROTOCOL.md.
//!
//! Linux-only runtime: on other hosts this binary only runs unit tests,
//! so host-unused items (uinput syscalls, socket code) are expected.
//! (Crate-level allow scoped to non-Linux builds.)
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

// vsock connect/serve only exist on Linux; on macOS this crate still
// compiles (and runs sysinfo unit tests) with the module kept alive.
mod capture;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod connector;
mod input;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod sysinfo;

#[cfg(target_os = "linux")]
use pegoles_guest_proto::{
    encode_guest, parse_host_message, Framer, GuestMessage, HostMessage, MessageError,
    GUEST_PROTOCOL_VERSION, RUNTIME_VERSION,
};
#[cfg(target_os = "linux")]
use std::io::{BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
mod vsock {
    use crate::connector::GuestEndpoint;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    fn vm_addr(cid: u32, port: u32) -> libc::sockaddr_vm {
        // SAFETY: sockaddr_vm is plain old data; zero is a valid value.
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as u16;
        addr.svm_cid = cid;
        addr.svm_port = port;
        addr
    }

    /// Hyper-V: listen on the privileged port. Binding it needs
    /// CAP_NET_BIND_SERVICE, which only the runtime's unit grants, so no
    /// other guest process can stand in for the runtime. Close-on-exec.
    pub fn listen(port: u32) -> std::io::Result<OwnedFd> {
        // SAFETY: plain socket(2).
        let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh, owned socket.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let local = vm_addr(libc::VMADDR_CID_ANY, port);
        let len = std::mem::size_of::<libc::sockaddr_vm>() as u32;
        // SAFETY: valid fd and a correctly sized sockaddr_vm.
        let ret = unsafe {
            libc::bind(
                owned.as_raw_fd(),
                &local as *const _ as *const libc::sockaddr,
                len,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // One host at a time; a second connection waits (or replaces the
        // first once it ends).
        // SAFETY: valid, bound socket.
        if unsafe { libc::listen(owned.as_raw_fd(), 1) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(owned)
    }

    /// Next host connection on a listener (blocks). Only the host can
    /// reach a guest vsock listener; peers are checked by the caller.
    pub fn accept(listener: &OwnedFd) -> std::io::Result<OwnedFd> {
        let mut peer: libc::sockaddr_vm = vm_addr(0, 0);
        let mut len = std::mem::size_of::<libc::sockaddr_vm>() as u32;
        // SAFETY: valid listener, correctly sized out-parameter.
        let fd = unsafe {
            libc::accept4(
                listener.as_raw_fd(),
                &mut peer as *mut _ as *mut libc::sockaddr,
                &mut len,
                libc::SOCK_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: accept4 returned a new, owned descriptor.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        // Guests only ever hear from their host (CID 2) over hv_sock;
        // anything else is refused.
        if peer.svm_cid != libc::VMADDR_CID_HOST {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("vsock peer is not the host (cid {})", peer.svm_cid),
            ));
        }
        Ok(owned)
    }

    /// AF_VSOCK connect to the host endpoint from a RESERVED source
    /// port. The host accepts only ports <= GUEST_SOURCE_PORT_MAX, which
    /// Linux lets only CAP_NET_BIND_SERVICE holders bind: that capability
    /// is what authenticates this process as the runtime.
    pub fn connect(endpoint: GuestEndpoint) -> std::io::Result<OwnedFd> {
        use pegoles_guest_proto::{GUEST_SOURCE_PORT_MAX, GUEST_SOURCE_PORT_MIN};
        let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh, owned socket (or we return early above).
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let len = std::mem::size_of::<libc::sockaddr_vm>() as u32;
        let mut bound = Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
        for source in (GUEST_SOURCE_PORT_MIN..=GUEST_SOURCE_PORT_MAX).rev() {
            let local = vm_addr(libc::VMADDR_CID_ANY, source);
            // SAFETY: valid fd and a correctly sized sockaddr_vm.
            let ret = unsafe {
                libc::bind(
                    owned.as_raw_fd(),
                    &local as *const _ as *const libc::sockaddr,
                    len,
                )
            };
            if ret == 0 {
                bound = Ok(());
                break;
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::AddrInUse {
                // EACCES: the unit did not grant CAP_NET_BIND_SERVICE.
                bound = Err(err);
                break;
            }
        }
        bound?;
        let remote = vm_addr(endpoint.host_cid, endpoint.port);
        // SAFETY: valid fd and a correctly sized sockaddr_vm.
        let ret = unsafe {
            libc::connect(
                owned.as_raw_fd(),
                &remote as *const _ as *const libc::sockaddr,
                len,
            )
        };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(owned)
    }
}

/// Where the next control connection comes from: dialled out to the
/// host, or accepted from it on the privileged listener.
#[cfg(target_os = "linux")]
enum Link {
    Dial(connector::GuestEndpoint),
    Listen(std::os::fd::OwnedFd),
}

#[cfg(target_os = "linux")]
impl Link {
    fn open(mode: connector::Mode) -> std::io::Result<Self> {
        Ok(match mode {
            connector::Mode::Dial(endpoint) => Link::Dial(endpoint),
            connector::Mode::Listen { port } => Link::Listen(vsock::listen(port)?),
        })
    }

    fn next(&self) -> std::io::Result<std::os::fd::OwnedFd> {
        match self {
            Link::Dial(endpoint) => vsock::connect(*endpoint),
            Link::Listen(listener) => vsock::accept(listener),
        }
    }
}

/// Connection wrapper: whatever ends the session (close, error,
/// protocol violation), held guest input is released first so the
/// compositor never keeps a stuck button or modifier.
#[cfg(target_os = "linux")]
fn serve_once(
    link: &Link,
    device: &mut Option<input::device::AgentDevice>,
    open_error: &Option<String>,
    flips: &mut FlipBudget,
) -> std::io::Result<()> {
    let result = link
        .next()
        .and_then(|conn| serve_inner(conn, device, open_error, flips));
    if let Some(dev) = device.as_ref() {
        dev.release_all();
    }
    result
}

/// Strict capability advertisement (Phase 5.1): a capability is listed
/// ONLY with a passing self-test; otherwise it lands in `unavailable`
/// with the structured reason. Pure (host-tested).
fn advertise(
    input: &Result<(), String>,
    capture: &Result<(), String>,
) -> (Vec<String>, Vec<pegoles_guest_proto::CapabilityDiagnostic>) {
    let mut capabilities = Vec::new();
    let mut unavailable = Vec::new();
    match input {
        Ok(()) => capabilities.push("input".to_string()),
        Err(reason) => unavailable.push(pegoles_guest_proto::CapabilityDiagnostic {
            capability: "input".to_string(),
            reason: reason.clone(),
        }),
    }
    match capture {
        Ok(()) => capabilities.push("frame".to_string()),
        Err(reason) => unavailable.push(pegoles_guest_proto::CapabilityDiagnostic {
            capability: "frame".to_string(),
            reason: reason.clone(),
        }),
    }
    (capabilities, unavailable)
}

/// Graphical session report from the connect-time trial capture. Pure
/// (host-tested): a captured frame proves the compositor is up and gives
/// its real output size; a failed probe is reported with its reason.
fn session_report(
    probe: &Result<(u32, u32), String>,
) -> pegoles_guest_proto::GraphicalSessionReport {
    use pegoles_guest_proto::{
        GraphicalSessionReport, GraphicalSessionStatus, MAX_SESSION_FIELD_BYTES,
    };
    match probe {
        Ok((w, h)) => GraphicalSessionReport {
            status: GraphicalSessionStatus::Ready,
            compositor: "weston".to_string(),
            width_px: Some(*w),
            height_px: Some(*h),
            detail: None,
        },
        Err(reason) => {
            let mut detail = reason.clone();
            if detail.len() > MAX_SESSION_FIELD_BYTES {
                let mut cut = MAX_SESSION_FIELD_BYTES;
                while !detail.is_char_boundary(cut) {
                    cut -= 1;
                }
                detail.truncate(cut);
            }
            GraphicalSessionReport {
                status: GraphicalSessionStatus::Unavailable,
                compositor: "weston".to_string(),
                width_px: None,
                height_px: None,
                detail: Some(detail),
            }
        }
    }
}

/// Reconnect budget for capability flips (Phase 5.1 monitor).
/// Heartbeat wakeups re-check cheap signals; a flip means the hello
/// lied about the present (weston arriving late, compositor dying,
/// device appearing). Reconnecting re-handshakes with fresh full
/// probes. Token bucket (1 flip per 60 s, burst 3): persistent
/// disagreement converges to quiet instead of reconnect-looping
/// forever, while transients always self-heal.
struct FlipBudget {
    flips: u32,
    last_flip_ms: u64,
}

const MAX_FLIPS: u32 = 3;
const FLIP_REFILL_MS: u64 = 60_000;

impl FlipBudget {
    fn new() -> Self {
        Self {
            flips: 0,
            last_flip_ms: 0,
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// True when `current` differs from `hello` and budget remains.
    fn should_reconnect(&mut self, hello: &[String], current: &[String]) -> bool {
        if hello == current {
            return false;
        }
        let now = Self::now_ms();
        if self.flips >= MAX_FLIPS {
            // Refill one credit per minute of quiet disagreement.
            if now.saturating_sub(self.last_flip_ms) < FLIP_REFILL_MS {
                return false;
            }
            self.flips = MAX_FLIPS - 1;
        }
        self.flips += 1;
        self.last_flip_ms = now;
        true
    }
}

/// Cheap capability signals for monitor wakeups (Linux): device
/// open-state + recognition (file reads) and compositor liveness
/// (registry roundtrip — milliseconds, never pixels). Full trial
/// capture stays connect-time only.
#[cfg(target_os = "linux")]
fn cheap_probe(device: &Option<input::device::AgentDevice>) -> Vec<String> {
    let mut caps = Vec::new();
    let input_ok = device.is_some()
        && std::fs::read_to_string("/proc/bus/input/devices")
            .map(|t| input::device_recognized(&t, input::DEVICE_NAME))
            .unwrap_or(false);
    if input_ok {
        caps.push("input".to_string());
    }
    if capture::query_capture_global().unwrap_or(false) {
        caps.push("frame".to_string());
    }
    caps
}
#[cfg(target_os = "linux")]
fn probe_input(
    device: &Option<input::device::AgentDevice>,
    open_error: &Option<String>,
) -> Result<(), String> {
    if device.is_none() {
        return Err(open_error.clone().unwrap_or_else(|| {
            "uinput device not opened (/dev/uinput inaccessible — image needs udev rule + input group)"
                .to_string()
        }));
    }
    for _ in 0..20 {
        match std::fs::read_to_string("/proc/bus/input/devices") {
            Ok(text) if input::device_recognized(&text, input::DEVICE_NAME) => return Ok(()),
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(format!("/proc/bus/input/devices unreadable: {e}")),
        }
    }
    Err("device created but not recognized by the input stack after 2s".to_string())
}

/// Self-test: Wayland socket visible AND one trial frame completes.
/// Pixels are discarded; success proves the capture path works.
/// Bounded (15 s wall clock): a stalled compositor must not wedge the
/// serve loop past the host heartbeat budget.
#[cfg(target_os = "linux")]
fn probe_capture() -> Result<(u32, u32), String> {
    if capture::display_socket_path().is_none() {
        return Err("no Wayland socket (compositor not running?)".to_string());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(capture::capture());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(15)) {
        Ok(Ok((w, h, pixels))) => {
            if w == 0 || h == 0 || pixels.len() != w as usize * h as usize * 4 {
                return Err("trial capture failed stride check".to_string());
            }
            Ok((w, h))
        }
        Ok(Err(e)) => Err(format!("trial capture failed: {e}")),
        Err(_) => Err("trial capture timed out after 15s".to_string()),
    }
}

#[cfg(target_os = "linux")]
fn execute_input(
    device: &Option<input::device::AgentDevice>,
    request_id: &str,
    op: &pegoles_guest_proto::GuestInputOp,
    display: Option<pegoles_guest_proto::GuestDisplaySize>,
) -> Result<(), String> {
    if request_id.len() > pegoles_guest_proto::MAX_REQUEST_ID_BYTES {
        return Err("request id exceeds bound".to_string());
    }
    let dev = device
        .as_ref()
        .ok_or_else(|| "input device unavailable (image needs /dev/uinput access)".to_string())?;
    let display = display.ok_or_else(|| "input needs display size".to_string())?;
    if display.width_px == 0 || display.height_px == 0 {
        return Err("absurd display size".to_string());
    }
    dev.execute(op, display)
}

/// One connection lifetime: handshake then serve until EOF/error.
/// Returns `Ok(())` only to signal "reconnect and try again".
/// `flips` persists across connections (reconnect budget converges).
#[cfg(target_os = "linux")]
fn serve_inner(
    owned: std::os::fd::OwnedFd,
    device: &mut Option<input::device::AgentDevice>,
    open_error: &Option<String>,
    flips: &mut FlipBudget,
) -> std::io::Result<()> {
    use std::os::fd::{FromRawFd, IntoRawFd};
    let fd = owned.into_raw_fd();
    // SAFETY: fd is ours; wrap read + write halves separately via dup.
    let read_fd = unsafe { libc::dup(fd) };
    if read_fd < 0 {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error());
    }
    let mut reader = BufReader::new(unsafe { std::fs::File::from_raw_fd(read_fd) });
    let mut writer = unsafe { std::fs::File::from_raw_fd(fd) };
    // Heartbeat-class wakeups for the capability monitor: a quiet
    // connection still re-checks cheap signals every 2 s (never pixels:
    // file reads + one Wayland registry roundtrip). Uses the raw fd
    // (BufReader has no timeout API).
    {
        use std::os::fd::AsRawFd;
        let tv = libc::timeval {
            tv_sec: 2,
            tv_usec: 0,
        };
        unsafe {
            libc::setsockopt(
                reader.get_ref().as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &tv as *const _ as *const libc::c_void,
                std::mem::size_of_val(&tv) as libc::socklen_t,
            );
        }
    }

    let mut framer = Framer::new();
    let mut buf = [0u8; 8192];
    let mut greeted = false;
    let mut ready_sent = false;

    // 1. Guest initiates: GuestHello first, always. STRICT (Phase
    // 5.1): capabilities are self-tested per connection — device
    // created AND recognized; trial frame captured. Anything failing
    // lands in `unavailable` with its reason, never in `capabilities`.
    let capture_probe = probe_capture();
    let capture_ok = capture_probe.as_ref().map(|_| ()).map_err(Clone::clone);
    let (capabilities, unavailable) = advertise(&probe_input(device, open_error), &capture_ok);
    send(
        &mut writer,
        &GuestMessage::GuestHello {
            protocol_version: GUEST_PROTOCOL_VERSION,
            runtime_version: RUNTIME_VERSION.to_string(),
            os: "debian".into(),
            os_version: debian_version(),
            arch: std::env::consts::ARCH.into(),
            capabilities: capabilities.clone(),
            unavailable,
        },
    )?;

    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Heartbeat-class wakeup (no host traffic): cheap
                // capability check (file reads + registry roundtrip —
                // never pixels). On a budgeted flip, reconnect so the
                // next hello advertises the truth (e.g. weston arriving
                // late on first boot, or dying later).
                if flips.should_reconnect(&capabilities, &cheap_probe(device)) {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(()); // host closed: reconnect
        }
        let frames = framer.push(&buf[..n]).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("framing: {e:?}"))
        })?;
        for line in frames {
            match parse_host_message(&line) {
                Ok(HostMessage::HostHello { protocol_version }) => {
                    if protocol_version != GUEST_PROTOCOL_VERSION {
                        // Host too new/old for us: report and drop; backoff retry.
                        let _ = send(
                            &mut writer,
                            &GuestMessage::Error {
                                code: "incompatible".into(),
                                message: format!("host protocol {protocol_version}"),
                            },
                        );
                        return Ok(());
                    }
                    greeted = true;
                    send(&mut writer, &GuestMessage::Ready)?;
                    ready_sent = true;
                }
                Ok(HostMessage::Ping { nonce }) => {
                    if greeted {
                        send(&mut writer, &GuestMessage::Pong { nonce })?;
                    }
                }
                Ok(HostMessage::GetSystemInfo) => {
                    if greeted {
                        send(&mut writer, &GuestMessage::SystemInfo(sysinfo::collect()))?;
                    }
                }
                Ok(HostMessage::Error { .. }) => {
                    // Host reports a problem: drop and reconnect fresh.
                    return Ok(());
                }
                Ok(HostMessage::GetGraphicalSession) => {
                    // Ready only when this connection's trial frame was
                    // captured from the compositor (real pixels, real size).
                    if greeted {
                        send(
                            &mut writer,
                            &GuestMessage::GraphicalSession(session_report(&capture_probe)),
                        )?;
                    }
                }
                Ok(HostMessage::Input {
                    request_id,
                    op,
                    display,
                }) => {
                    if !greeted {
                        continue;
                    }
                    let ack = match execute_input(device, &request_id, &op, display) {
                        Ok(()) => GuestMessage::InputAck {
                            request_id: request_id.clone(),
                            ok: true,
                            error: None,
                        },
                        Err(error) => GuestMessage::InputAck {
                            request_id: request_id.clone(),
                            ok: false,
                            error: Some(error),
                        },
                    };
                    send(&mut writer, &ack)?;
                }
                Ok(HostMessage::GetFrame { request_id }) => {
                    if !greeted {
                        continue;
                    }
                    // Capture failures ride the InputAck channel (same
                    // request id, ok=false) so the host fails fast instead
                    // of timing out a 30 s transfer.
                    match capture::capture() {
                        // `capture()` already returns RGBA (see capture.rs).
                        Ok((width_px, height_px, pixels)) => {
                            let total = match capture::chunk_count(pixels.len()) {
                                Ok(n) => n,
                                Err(e) => {
                                    send(
                                        &mut writer,
                                        &GuestMessage::InputAck {
                                            request_id: request_id.clone(),
                                            ok: false,
                                            error: Some(e),
                                        },
                                    )?;
                                    continue;
                                }
                            };
                            send(
                                &mut writer,
                                &GuestMessage::FrameBegin {
                                    request_id: request_id.clone(),
                                    width_px,
                                    height_px,
                                    total_chunks: total,
                                },
                            )?;
                            for (seq, raw) in pixels.chunks(32 * 1024).enumerate() {
                                send(
                                    &mut writer,
                                    &GuestMessage::FrameChunk {
                                        request_id: request_id.clone(),
                                        seq: seq as u32,
                                        bytes: capture::base64_encode(raw),
                                    },
                                )?;
                            }
                        }
                        Err(error) => {
                            send(
                                &mut writer,
                                &GuestMessage::InputAck {
                                    request_id: request_id.clone(),
                                    ok: false,
                                    error: Some(error),
                                },
                            )?;
                        }
                    }
                }
                Err(MessageError::UnknownType) => {
                    // Forward compatibility: ignore unknown host messages.
                    continue;
                }
                Err(_) => {
                    // Malformed frame: drop the connection, backoff retry.
                    return Ok(());
                }
            }
        }
        let _ = ready_sent;
    }
}

#[cfg(target_os = "linux")]
fn send(writer: &mut std::fs::File, msg: &GuestMessage) -> std::io::Result<()> {
    let mut line = encode_guest(msg);
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

#[cfg(target_os = "linux")]
fn debian_version() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|c| {
            c.lines().find_map(|l| {
                l.strip_prefix("VERSION_ID=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!(
            "pegoles-guest-runtime {} (guest protocol {})",
            pegoles_guest_proto::RUNTIME_VERSION,
            pegoles_guest_proto::GUEST_PROTOCOL_VERSION
        );
        return;
    }
    // Not dumpable: other processes of the same guest user (GUI apps)
    // cannot ptrace us or read our memory to borrow the reserved-port
    // capability or the live control connection.
    // SAFETY: prctl with integer arguments only.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    // The uinput device lives for the whole process (reused across
    // reconnects). Open lazily: a missing /dev/uinput simply means no
    // "input" capability is advertised — never a crash.
    let mut device: Option<input::device::AgentDevice> = None;
    let mut input_warned = false;
    let mut open_error: Option<String> = None;
    // Persistent across connections: capability-flip reconnect budget
    // converges instead of looping on persistent disagreement.
    let mut flips = FlipBudget::new();
    // First boot: the compositor usually starts a moment after us. Give
    // it a bounded head start so the first hello can already advertise
    // frame capture (otherwise the host waits for a capability flip).
    wait_for_compositor(Duration::from_secs(15));
    let args: Vec<String> = std::env::args().collect();
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let mode = connector::resolve_mode(&args, &cmdline);
    // A listener that cannot be opened (no capability, no hv_sock) is
    // retried: the unit restarts us, and the host sees no runtime.
    let link = loop {
        match Link::open(mode) {
            Ok(link) => break link,
            Err(e) => {
                eprintln!("pegoles-guest-runtime: cannot open the control channel ({mode:?}): {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    // Capped backoff: 1,2,4,5,5…s. Retries continue while the
    // process lives; systemd Restart=on-failure covers real crashes.
    // Exit codes: 0 only on clean shutdown paths (none currently — the
    // loop is infinite by design; SIGTERM from systemd ends us).
    let mut backoff = Duration::from_secs(1);
    loop {
        if device.is_none() {
            match input::device::open() {
                Ok(dev) => {
                    device = Some(dev);
                    open_error = None;
                }
                Err(e) => {
                    open_error = Some(e.clone());
                    // Log once: retrying every backoff tick would spam.
                    if !input_warned {
                        eprintln!("pegoles-guest-runtime: {e}");
                        input_warned = true;
                    }
                }
            }
        }
        let started = std::time::Instant::now();
        match serve_once(&link, &mut device, &open_error, &mut flips) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("pegoles-guest-runtime: connection failed: {e}");
            }
        }
        // A long-lived connection means the setup works: retry fast next
        // time. Quick failures back off exponentially, capped at 5 s so a
        // runtime restart reconnects within seconds.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        } else {
            backoff = std::cmp::min(backoff * 2, Duration::from_secs(5));
        }
        std::thread::sleep(backoff);
    }
}

/// Wait until the Wayland socket exists, at most `max`. Returns early on
/// headless images (no WAYLAND_DISPLAY configured).
#[cfg(target_os = "linux")]
fn wait_for_compositor(max: Duration) {
    let Some(path) = capture::display_socket_path() else {
        return;
    };
    let deadline = std::time::Instant::now() + max;
    while !std::path::Path::new(&path).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("pegoles-guest-runtime runs on Linux only (guest VM side)");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_lists_only_passing_self_tests() {
        let (caps, un) = advertise(&Ok(()), &Ok(()));
        assert_eq!(caps, vec!["input".to_string(), "frame".to_string()]);
        assert!(un.is_empty());

        let (caps, un) = advertise(&Err("no uinput".to_string()), &Ok(()));
        assert_eq!(caps, vec!["frame".to_string()]);
        assert_eq!(un.len(), 1);
        assert_eq!(un[0].capability, "input");
        assert!(un[0].reason.contains("uinput"));

        let (caps, un) = advertise(&Err("a".to_string()), &Err("b".to_string()));
        assert!(caps.is_empty());
        assert_eq!(un.len(), 2);
    }

    #[test]
    fn session_report_follows_trial_capture() {
        let ready = session_report(&Ok((1440, 900)));
        assert_eq!(
            ready.status,
            pegoles_guest_proto::GraphicalSessionStatus::Ready
        );
        assert_eq!((ready.width_px, ready.height_px), (Some(1440), Some(900)));
        assert!(ready.is_bounded());
        let down = session_report(&Err("x".repeat(1000)));
        assert_eq!(
            down.status,
            pegoles_guest_proto::GraphicalSessionStatus::Unavailable
        );
        assert!(down.is_bounded());
    }

    #[test]
    fn flip_budget_converges() {
        let hello = vec!["input".to_string()];
        let mut budget = FlipBudget::new();
        // Same state: no reconnect.
        assert!(!budget.should_reconnect(&hello, &hello));
        // Flip: reconnect (weston arriving late, compositor dying).
        let changed = vec!["input".to_string(), "frame".to_string()];
        assert!(budget.should_reconnect(&hello, &changed));
        assert!(budget.should_reconnect(&hello, &changed));
        assert!(budget.should_reconnect(&hello, &changed));
        // Budget exhausted: quiet instead of looping forever.
        assert!(!budget.should_reconnect(&hello, &changed));
        // Refill after a quiet minute: transients self-heal.
        budget.last_flip_ms = FlipBudget::now_ms().saturating_sub(61_000);
        assert!(budget.should_reconnect(&hello, &changed));
    }
}

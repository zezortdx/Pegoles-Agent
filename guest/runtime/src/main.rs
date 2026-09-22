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
    GUEST_PROTOCOL_VERSION, PEGOLES_VSOCK_PORT, RUNTIME_VERSION,
};
#[cfg(target_os = "linux")]
use std::io::{BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
mod vsock {
    use crate::connector::resolve_host_endpoint;

    /// AF_VSOCK connect to the resolved host endpoint.
    pub fn connect(port: u32) -> std::io::Result<std::os::fd::OwnedFd> {
        use std::os::fd::FromRawFd;
        let endpoint = resolve_host_endpoint();
        debug_assert_eq!(port, endpoint.port);
        let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh, owned socket (or we return early above).
        let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as u16;
        addr.svm_cid = endpoint.host_cid;
        addr.svm_port = endpoint.port;
        let ret = unsafe {
            libc::connect(
                owned.as_raw_fd(),
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_vm>() as u32,
            )
        };
        use std::os::fd::AsRawFd;
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(owned)
    }
}

/// Connection wrapper: whatever ends the session (close, error,
/// protocol violation), held guest input is released first so the
/// compositor never keeps a stuck button or modifier.
#[cfg(target_os = "linux")]
fn serve_once(device: &mut Option<input::device::AgentDevice>) -> std::io::Result<()> {
    let result = serve_inner(device);
    if let Some(dev) = device.as_ref() {
        dev.release_all();
    }
    result
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
#[cfg(target_os = "linux")]
fn serve_inner(device: &mut Option<input::device::AgentDevice>) -> std::io::Result<()> {
    use std::os::fd::{FromRawFd, IntoRawFd};
    let owned = vsock::connect(PEGOLES_VSOCK_PORT)?;
    let fd = owned.into_raw_fd();
    // SAFETY: fd is ours; wrap read + write halves separately via dup.
    let read_fd = unsafe { libc::dup(fd) };
    if read_fd < 0 {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error());
    }
    let mut reader = BufReader::new(unsafe { std::fs::File::from_raw_fd(read_fd) });
    let mut writer = unsafe { std::fs::File::from_raw_fd(fd) };

    let mut framer = Framer::new();
    let mut buf = [0u8; 8192];
    let mut greeted = false;
    let mut ready_sent = false;

    // 1. Guest initiates: GuestHello first, always. Capabilities are
    // probed honestly: input iff the uinput device opened, frame iff a
    // Wayland socket is visible.
    let mut capabilities = Vec::new();
    if device.is_some() {
        capabilities.push("input".to_string());
    }
    if capture::display_socket_path().is_some() {
        capabilities.push("frame".to_string());
    }
    send(
        &mut writer,
        &GuestMessage::GuestHello {
            protocol_version: GUEST_PROTOCOL_VERSION,
            runtime_version: RUNTIME_VERSION.to_string(),
            os: "debian".into(),
            os_version: debian_version(),
            arch: std::env::consts::ARCH.into(),
            capabilities,
        },
    )?;

    loop {
        let n = reader.read(&mut buf)?;
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
                    // CONTRACT STUB (Phase 4): the guest-image stream
                    // implements the real compositor probe. Until then,
                    // report honestly that no session is known.
                    if greeted {
                        send(
                            &mut writer,
                            &GuestMessage::GraphicalSession(
                                pegoles_guest_proto::GraphicalSessionReport {
                                    status:
                                        pegoles_guest_proto::GraphicalSessionStatus::Unavailable,
                                    compositor: String::new(),
                                    width_px: None,
                                    height_px: None,
                                    detail: None,
                                },
                            ),
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
                        Ok((width_px, height_px, mut pixels)) => {
                            capture::argb_to_rgba(&mut pixels);
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
    // The uinput device lives for the whole process (reused across
    // reconnects). Open lazily: a missing /dev/uinput simply means no
    // "input" capability is advertised — never a crash.
    let mut device: Option<input::device::AgentDevice> = None;
    let mut input_warned = false;
    // Capped backoff: 1,2,4,8,15,30,30…s. Retries continue while the
    // process lives; systemd Restart=on-failure covers real crashes.
    // Exit codes: 0 only on clean shutdown paths (none currently — the
    // loop is infinite by design; SIGTERM from systemd ends us).
    let mut backoff = Duration::from_secs(1);
    loop {
        if device.is_none() {
            match input::device::open() {
                Ok(dev) => device = Some(dev),
                Err(e) => {
                    // Log once: retrying every backoff tick would spam.
                    if !input_warned {
                        eprintln!("pegoles-guest-runtime: {e}");
                        input_warned = true;
                    }
                }
            }
        }
        let started = std::time::Instant::now();
        match serve_once(&mut device) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("pegoles-guest-runtime: connection failed: {e}");
            }
        }
        // A long-lived connection means the setup works: retry fast next
        // time. Quick failures back off exponentially, capped at 30 s.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        } else {
            backoff = std::cmp::min(backoff * 2, Duration::from_secs(30));
        }
        std::thread::sleep(backoff);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("pegoles-guest-runtime runs on Linux only (guest VM side)");
    std::process::exit(1);
}

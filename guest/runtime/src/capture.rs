//! Guest framebuffer capture (Phase 5).
//!
//! A minimal `weston-screenshooter` Wayland client written against the raw
//! wire protocol with std + libc only (no wayland-client link needed in
//! the guest). Captures the guest output into shared memory, shuffles
//! ARGB8888 to RGBA, and emits base64 chunks for the vsock transport.
//!
//! Layout mirrors `input.rs`: pure logic (wire marshalling, pixel
//! shuffle, base64, chunk planning) is platform-free and host-tested;
//! socket fd-passing and memfd live behind the Linux-gated `capture`
//! function. If the compositor does not expose `weston_screenshooter`,
//! capture fails with a structured honest error (flag/image side).

use pegoles_guest_proto::{MAX_FRAME_CHUNKS, MAX_FRAME_CHUNK_B64};

/// WAYLAND_DISPLAY resolution: explicit env, else the user runtime dir.
pub fn display_socket_path() -> Option<String> {
    if let Ok(explicit) = std::env::var("WAYLAND_DISPLAY") {
        if explicit.starts_with('/') {
            return Some(explicit);
        }
        if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            return Some(format!("{runtime}/{explicit}"));
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(format!("{home}/.local/share/wayland/{explicit}"));
        }
        return Some(format!("/run/user/1000/{explicit}"));
    }
    for candidate in ["/run/user/1000/wayland-0", "/run/user/1001/wayland-0"] {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}

// --- pure wire marshalling -------------------------------------------------

/// Append a padding-aligned Wayland string (len+NUL, 4-aligned).
pub fn put_string(out: &mut Vec<u8>, s: &str) {
    let len = s.len() as u32 + 1;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Frame one request: header (object, size|opcode) + payload.
pub fn frame_request(object: u32, opcode: u16, payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u16;
    let mut out = Vec::with_capacity(size as usize);
    put_u32(&mut out, object);
    out.extend_from_slice(&((size as u32) << 16 | opcode as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn read_u32(data: &[u8], at: &mut usize) -> Option<u32> {
    let b: [u8; 4] = data.get(*at..*at + 4)?.try_into().ok()?;
    *at += 4;
    Some(u32::from_le_bytes(b))
}

fn read_i32(data: &[u8], at: &mut usize) -> Option<i32> {
    read_u32(data, at).map(|v| v as i32)
}

fn read_string(data: &[u8], at: &mut usize) -> Option<String> {
    let len = read_u32(data, at)? as usize;
    if len == 0 || len > data.len().saturating_sub(*at) + 4 {
        return None;
    }
    let bytes = data.get(*at..*at + len - 1)?;
    *at += len - 1;
    if data.get(*at) != Some(&0) {
        return None;
    }
    *at += 1;
    while !(*at).is_multiple_of(4) {
        if data.get(*at) != Some(&0) {
            return None;
        }
        *at += 1;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// One parsed Wayland event (sender object, opcode, arg payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireEvent {
    pub sender: u32,
    pub opcode: u16,
    pub args: Vec<u8>,
}

/// Parse all complete events from a byte buffer; returns events plus
/// leftover bytes. Malformed headers stop parsing (caller errors).
pub fn parse_events(mut data: &[u8]) -> (Vec<WireEvent>, Vec<u8>) {
    let mut events = Vec::new();
    while data.len() >= 8 {
        let sender = u32::from_le_bytes(data[0..4].try_into().expect("len"));
        let size_op = u32::from_le_bytes(data[4..8].try_into().expect("len"));
        let size = (size_op >> 16) as usize;
        let opcode = (size_op & 0xffff) as u16;
        if size < 8 || size > data.len() + 8 {
            break;
        }
        if size > data.len() {
            break;
        }
        events.push(WireEvent {
            sender,
            opcode,
            args: data[8..size].to_vec(),
        });
        data = &data[size..];
    }
    (events, data.to_vec())
}

/// Registry global announcement (opcode 0 on wl_registry).
pub fn parse_global(args: &[u8]) -> Option<(u32, String, u32)> {
    let mut at = 0;
    let name = read_u32(args, &mut at)?;
    let interface = read_string(args, &mut at)?;
    let version = read_u32(args, &mut at)?;
    Some((name, interface, version))
}

/// wl_output mode event (opcode 1): flags, width, height, refresh.
pub fn parse_output_mode(args: &[u8]) -> Option<(i32, i32)> {
    let mut at = 0;
    let _flags = read_u32(args, &mut at)?;
    let width = read_i32(args, &mut at)?;
    let height = read_i32(args, &mut at)?;
    Some((width, height))
}

/// wl_display error event (opcode 0): object id, code, message.
pub fn parse_display_error(args: &[u8]) -> Option<(u32, u32, String)> {
    let mut at = 0;
    let object = read_u32(args, &mut at)?;
    let code = read_u32(args, &mut at)?;
    let message = read_string(args, &mut at)?;
    Some((object, code, message))
}

// --- pure pixel + chunk helpers --------------------------------------------

/// ARGB8888 (little-endian memory: B,G,R,A) -> RGBA in place.
pub fn argb_to_rgba(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Minimal base64 (no new dependencies in the guest).
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Plan chunking: number of 32 KiB raw chunks for `byte_len` (cap-checked).
pub fn chunk_count(byte_len: usize) -> Result<u32, String> {
    const RAW: usize = 32 * 1024;
    let n = byte_len.div_ceil(RAW) as u64;
    if n == 0 || n > MAX_FRAME_CHUNKS as u64 {
        return Err(format!("frame size {byte_len} outside chunk plan"));
    }
    // Each chunk must fit the guest frame cap once encoded.
    if RAW.div_ceil(3) * 4 > MAX_FRAME_CHUNK_B64 {
        return Err("chunk plan exceeds frame cap".to_string());
    }
    Ok(n as u32)
}

// --- Linux capture ----------------------------------------------------------

/// Abstract byte stream so the protocol dance is testable without a
/// compositor (fake impl) or sockets (real impl, Linux only).
pub trait CaptureStream {
    /// Send one request, attaching file descriptors when present.
    fn send(&mut self, bytes: &[u8], fds: &[i32]) -> Result<(), String>;
    /// Read available bytes (may be partial; caller buffers).
    fn recv(&mut self) -> Result<Vec<u8>, String>;
}

/// Registry globals of interest after binding.
struct Globals {
    shm: Option<(u32, u32)>,
    output: Option<(u32, u32)>,
    shooter: Option<(u32, u32)>,
}

fn collect_globals(events: &[WireEvent], registry: u32) -> Globals {
    let mut g = Globals {
        shm: None,
        output: None,
        shooter: None,
    };
    for e in events {
        if e.sender != registry || e.opcode != 0 {
            continue;
        }
        if let Some((name, interface, version)) = parse_global(&e.args) {
            match interface.as_str() {
                "wl_shm" if g.shm.is_none() => g.shm = Some((name, version.min(1))),
                "wl_output" if g.output.is_none() => g.output = Some((name, version.min(3))),
                "weston_screenshooter" if g.shooter.is_none() => {
                    g.shooter = Some((name, version.min(1)))
                }
                _ => {}
            }
        }
    }
    g
}

/// Drive the screenshooter dance over any stream. Returns raw RGBA.
pub fn capture_over<S: PoolStream>(stream: &mut S) -> Result<(u32, u32, Vec<u8>), String> {
    let mut next_id: u32 = 2;
    let mut fresh = || {
        let id = next_id;
        next_id += 1;
        id
    };
    let registry = fresh();
    // wl_display.get_registry
    let mut payload = Vec::new();
    put_u32(&mut payload, registry);
    stream.send(&frame_request(1, 1, &payload), &[])?;
    let callback = fresh();
    let mut payload = Vec::new();
    put_u32(&mut payload, callback);
    stream.send(&frame_request(1, 0, &payload), &[])?;
    let events = pump_until(stream, callback)?;
    let globals = collect_globals(&events, registry);
    let (shm_name, shm_version) = globals
        .shm
        .ok_or_else(|| "compositor lacks wl_shm".to_string())?;
    let (output_name, output_version) = globals
        .output
        .ok_or_else(|| "compositor has no wl_output".to_string())?;
    let (shooter_name, shooter_version) = globals.shooter.ok_or_else(|| {
        "compositor does not expose weston_screenshooter (needs the compositor capture flag)"
            .to_string()
    })?;
    // Bind the three globals.
    let shm = fresh();
    bind(stream, registry, shm_name, "wl_shm", shm_version, shm)?;
    let output = fresh();
    bind(
        stream,
        registry,
        output_name,
        "wl_output",
        output_version,
        output,
    )?;
    let shooter = fresh();
    bind(
        stream,
        registry,
        shooter_name,
        "weston_screenshooter",
        shooter_version,
        shooter,
    )?;
    // Roundtrip: output mode (size) arrives.
    let callback = fresh();
    let mut payload = Vec::new();
    put_u32(&mut payload, callback);
    stream.send(&frame_request(1, 0, &payload), &[])?;
    let events = pump_until(stream, callback)?;
    let (width, height) = events
        .iter()
        .filter(|e| e.sender == output && e.opcode == 1)
        .filter_map(|e| parse_output_mode(&e.args))
        .find(|(w, h)| *w > 0 && *h > 0)
        .map(|(w, h)| (w as u32, h as u32))
        .ok_or_else(|| "no output mode with dimensions".to_string())?;
    if width > 16384 || height > 16384 {
        return Err("absurd output dimensions".to_string());
    }
    // Pool + buffer, then shoot. The fd handoff is stream-specific:
    // `send_pool` returns the fd the pool was created with semantics the
    // stream understands (real: SCM_RIGHTS memfd; fake: marker only).
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| "stride overflow".to_string())?;
    let pool_size = stride
        .checked_mul(height)
        .ok_or_else(|| "pool overflow".to_string())?;
    let pool = fresh();
    let pool_fd = stream_send_pool(stream, shm, pool, pool_size)?;
    let buffer = fresh();
    let mut payload = Vec::new();
    put_u32(&mut payload, buffer);
    put_i32(&mut payload, 0);
    put_i32(&mut payload, width as i32);
    put_i32(&mut payload, height as i32);
    put_i32(&mut payload, stride as i32);
    put_u32(&mut payload, 0); // WL_SHM_FORMAT_ARGB8888
    stream.send(&frame_request(pool, 0, &payload), &[])?;
    // weston_screenshooter.shoot(output, buffer)
    let mut payload = Vec::new();
    put_u32(&mut payload, output);
    put_u32(&mut payload, buffer);
    stream.send(&frame_request(shooter, 0, &payload), &[])?;
    // Wait for shooter.done on the shooter object.
    pump_until_object(stream, shooter, 0)?;
    let pixels = stream_take_pixels(stream, pool_fd, pool_size)?;
    Ok((width, height, pixels))
}

fn bind<S: CaptureStream>(
    stream: &mut S,
    registry: u32,
    name: u32,
    interface: &str,
    version: u32,
    new_id: u32,
) -> Result<(), String> {
    let mut payload = Vec::new();
    put_u32(&mut payload, name);
    put_string(&mut payload, interface);
    put_u32(&mut payload, version);
    put_u32(&mut payload, new_id);
    stream.send(&frame_request(registry, 1, &payload), &[])?;
    Ok(())
}

/// Read until the callback-done event (sender == callback, opcode 0),
/// surfacing compositor errors. Returns all events seen.
fn pump_until<S: CaptureStream>(stream: &mut S, callback: u32) -> Result<Vec<WireEvent>, String> {
    let mut pending = Vec::new();
    let mut seen = Vec::new();
    for _ in 0..512 {
        let chunk = stream.recv()?;
        if chunk.is_empty() {
            return Err("compositor closed the connection".to_string());
        }
        pending.extend_from_slice(&chunk);
        let (events, rest) = parse_events(&pending);
        pending = rest;
        for e in events {
            if e.sender == 1 && e.opcode == 0 {
                let (_, code, message) = parse_display_error(&e.args).unwrap_or((0, 0, "?".into()));
                return Err(format!("compositor error {code}: {message}"));
            }
            if e.sender == callback && e.opcode == 0 {
                return Ok(seen);
            }
            seen.push(e);
        }
    }
    Err("roundtrip without callback done".to_string())
}

/// Read until a specific (sender, opcode) event arrives.
fn pump_until_object<S: CaptureStream>(
    stream: &mut S,
    sender: u32,
    opcode: u16,
) -> Result<(), String> {
    let mut pending = Vec::new();
    for _ in 0..512 {
        let chunk = stream.recv()?;
        if chunk.is_empty() {
            return Err("compositor closed the connection".to_string());
        }
        pending.extend_from_slice(&chunk);
        let (events, rest) = parse_events(&pending);
        pending = rest;
        for e in events {
            if e.sender == 1 && e.opcode == 0 {
                let (_, code, message) = parse_display_error(&e.args).unwrap_or((0, 0, "?".into()));
                return Err(format!("compositor error {code}: {message}"));
            }
            if e.sender == sender && e.opcode == opcode {
                return Ok(());
            }
        }
    }
    Err("capture without shooter done".to_string())
}

/// Default pool/pixel handling for streams without fd semantics (tests).
/// Real sockets override via trait methods below.
fn stream_send_pool<S: PoolStream>(
    stream: &mut S,
    shm: u32,
    pool: u32,
    pool_size: u32,
) -> Result<i32, String> {
    stream.send_pool(shm, pool, pool_size)
}

fn stream_take_pixels<S: PoolStream>(
    stream: &mut S,
    pool_fd: i32,
    pool_size: u32,
) -> Result<Vec<u8>, String> {
    stream.take_pixels(pool_fd, pool_size)
}

/// Stream capabilities: fd-backed shm pools (real sockets) or marker
/// fds with supplied pixels (test doubles). Implemented per stream:
/// `RealCapture` on Linux, fakes in tests.
pub trait PoolStream: CaptureStream {
    fn send_pool(&mut self, shm: u32, pool: u32, pool_size: u32) -> Result<i32, String>;

    fn take_pixels(&mut self, _pool_fd: i32, pool_size: u32) -> Result<Vec<u8>, String> {
        Ok(vec![0u8; pool_size as usize])
    }
}

/// Real Unix socket stream (Linux): SCM_RIGHTS fd passing + memfd pools.
#[cfg(target_os = "linux")]
pub mod socket {
    use super::CaptureStream;
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::os::unix::net::UnixStream;

    pub struct SocketStream {
        stream: UnixStream,
    }

    impl SocketStream {
        pub fn connect(path: &str) -> Result<Self, String> {
            let stream =
                UnixStream::connect(path).map_err(|e| format!("wayland connect {path}: {e}"))?;
            // Bounded reads: a stalled compositor must not hang capture.
            let timeout = libc_timeval(5, 0);
            let ret = unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVTIMEO,
                    &timeout as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&timeout) as libc::socklen_t,
                )
            };
            if ret != 0 {
                return Err(format!("setsockopt: {}", std::io::Error::last_os_error()));
            }
            Ok(Self { stream })
        }
    }

    fn libc_timeval(sec: i64, usec: i64) -> libc::timeval {
        libc::timeval {
            tv_sec: sec as libc::time_t,
            tv_usec: usec as libc::suseconds_t,
        }
    }

    impl CaptureStream for SocketStream {
        fn send(&mut self, bytes: &[u8], fds: &[i32]) -> Result<(), String> {
            if fds.is_empty() {
                use std::io::Write;
                self.stream
                    .write_all(bytes)
                    .map_err(|e| format!("wayland send: {e}"))?;
                return Ok(());
            }
            send_with_fds(self.stream.as_raw_fd(), bytes, fds)
        }

        fn recv(&mut self) -> Result<Vec<u8>, String> {
            let mut buf = [0u8; 65536];
            let n = unsafe {
                libc::recv(
                    self.stream.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                )
            };
            if n < 0 {
                return Err(format!("wayland recv: {}", std::io::Error::last_os_error()));
            }
            Ok(buf[..n as usize].to_vec())
        }
    }

    fn send_with_fds(fd: RawFd, bytes: &[u8], fds: &[i32]) -> Result<(), String> {
        use std::mem::{size_of, zeroed};
        unsafe {
            let mut iov = libc::iovec {
                iov_base: bytes.as_ptr() as *mut libc::c_void,
                iov_len: bytes.len(),
            };
            let cmsg_len = libc::CMSG_SPACE((size_of::<RawFd>() * fds.len()) as u32) as usize;
            let mut cmsg_buf = vec![0u8; cmsg_len];
            let mut hdr: libc::msghdr = zeroed();
            hdr.msg_iov = &mut iov;
            hdr.msg_iovlen = 1;
            hdr.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
            hdr.msg_controllen = cmsg_buf.len();
            let cmsg = libc::CMSG_FIRSTHDR(&hdr);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN((size_of::<RawFd>() * fds.len()) as u32) as _;
            let dst = libc::CMSG_DATA(cmsg) as *mut RawFd;
            for (i, f) in fds.iter().enumerate() {
                *dst.add(i) = *f;
            }
            let n = libc::sendmsg(fd, &hdr, 0);
            if n < 0 || n as usize != bytes.len() {
                return Err(format!("sendmsg: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }
    }

    /// Read back the memfd pool contents after shooter.done.
    pub fn read_pool(fd: RawFd, size: u32) -> Result<Vec<u8>, String> {
        let mut pixels = vec![0u8; size as usize];
        let mut got = 0;
        while got < pixels.len() {
            let n = unsafe {
                libc::pread(
                    fd,
                    pixels[got..].as_mut_ptr() as *mut libc::c_void,
                    pixels.len() - got,
                    got as i64,
                )
            };
            if n <= 0 {
                return Err(format!("pool read: {}", std::io::Error::last_os_error()));
            }
            got += n as usize;
        }
        Ok(pixels)
    }
}

/// High-level capture entry point (Linux): connect, shoot, shuffle.
#[cfg(target_os = "linux")]
pub fn capture() -> Result<(u32, u32, Vec<u8>), String> {
    let path = display_socket_path()
        .ok_or_else(|| "no wayland socket (WAYLAND_DISPLAY unset)".to_string())?;
    let stream = socket::SocketStream::connect(&path)?;
    // Wrap the socket so pool creation uses a real memfd + SCM_RIGHTS and
    // pixel readout reads the pool back.
    let mut real = RealCapture {
        stream,
        pool_fd: -1,
        pool_size: 0,
    };
    let (w, h, mut pixels) = capture_over(&mut real)?;
    argb_to_rgba(&mut pixels);
    Ok((w, h, pixels))
}

#[cfg(target_os = "linux")]
struct RealCapture {
    stream: socket::SocketStream,
    pool_fd: i32,
    pool_size: u32,
}

#[cfg(target_os = "linux")]
impl CaptureStream for RealCapture {
    fn send(&mut self, bytes: &[u8], fds: &[i32]) -> Result<(), String> {
        self.stream.send(bytes, fds)
    }

    fn recv(&mut self) -> Result<Vec<u8>, String> {
        self.stream.recv()
    }
}

#[cfg(target_os = "linux")]
impl PoolStream for RealCapture {
    fn send_pool(&mut self, shm: u32, pool: u32, pool_size: u32) -> Result<i32, String> {
        use crate::capture::{frame_request, put_i32, put_u32};
        // Real memfd + wl_shm.create_pool with SCM_RIGHTS in one step.
        let fd = unsafe { libc::memfd_create(b"pegoles-shot\0".as_ptr(), 1) };
        if fd < 0 {
            return Err(format!("memfd_create: {}", std::io::Error::last_os_error()));
        }
        if unsafe { libc::ftruncate(fd, pool_size as i64) } != 0 {
            unsafe { libc::close(fd) };
            return Err(format!("ftruncate: {}", std::io::Error::last_os_error()));
        }
        let mut payload = Vec::new();
        put_u32(&mut payload, pool);
        put_i32(&mut payload, pool_size as i32);
        self.stream.send(&frame_request(shm, 0, &payload), &[fd])?;
        self.pool_fd = fd;
        self.pool_size = pool_size;
        Ok(fd)
    }

    fn take_pixels(&mut self, _pool_fd: i32, pool_size: u32) -> Result<Vec<u8>, String> {
        socket::read_pool(self.pool_fd, pool_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted fake compositor: answers registry, binds, mode, done.
    struct FakeCompositor {
        inbox: Vec<(Vec<u8>, Vec<i32>)>,
        script: Vec<Vec<u8>>,
        pixels: Vec<u8>,
        pool_size: u32,
    }

    impl FakeCompositor {
        fn event(sender: u32, opcode: u16, args: &[u8]) -> Vec<u8> {
            let size = (8 + args.len()) as u16;
            let mut out = Vec::new();
            put_u32(&mut out, sender);
            out.extend_from_slice(&((size as u32) << 16 | opcode as u32).to_le_bytes());
            out.extend_from_slice(args);
            out
        }

        fn global(name: u32, interface: &str, version: u32) -> Vec<u8> {
            let mut args = Vec::new();
            put_u32(&mut args, name);
            put_string(&mut args, interface);
            put_u32(&mut args, version);
            Self::event(2, 0, &args)
        }

        fn new_mode() -> Self {
            let mut script = Vec::new();
            // First roundtrip: globals + callback done.
            let mut first = Vec::new();
            first.extend(Self::global(10, "wl_shm", 1));
            first.extend(Self::global(11, "wl_output", 3));
            first.extend(Self::global(12, "weston_screenshooter", 1));
            first.extend(Self::event(3, 0, &[])); // callback done
            script.push(first);
            // Second roundtrip: output mode + callback done.
            let mut mode_args = Vec::new();
            put_u32(&mut mode_args, 0); // flags
            put_i32(&mut mode_args, 64);
            put_i32(&mut mode_args, 36);
            put_i32(&mut mode_args, 60000);
            let mut second = Vec::new();
            second.extend(Self::event(5, 1, &mode_args)); // output id 5
            second.extend(Self::event(7, 0, &[])); // callback done
            script.push(second);
            // Shooter done.
            script.push(Self::event(6, 0, &[]));
            // 64x36 test pixels: ARGB (B,G,R,A memory order).
            let mut pixels = Vec::with_capacity(64 * 36 * 4);
            for i in 0..64 * 36 {
                pixels.push((i % 251) as u8);
                pixels.push(((i * 2) % 251) as u8);
                pixels.push(((i * 3) % 251) as u8);
                pixels.push(255);
            }
            Self {
                inbox: Vec::new(),
                script,
                pixels,
                pool_size: 0,
            }
        }
    }

    impl CaptureStream for FakeCompositor {
        fn send(&mut self, bytes: &[u8], fds: &[i32]) -> Result<(), String> {
            self.inbox.push((bytes.to_vec(), fds.to_vec()));
            Ok(())
        }

        fn recv(&mut self) -> Result<Vec<u8>, String> {
            if self.script.is_empty() {
                return Ok(Vec::new());
            }
            Ok(self.script.remove(0))
        }
    }

    impl PoolStream for FakeCompositor {
        fn send_pool(&mut self, _shm: u32, _pool: u32, pool_size: u32) -> Result<i32, String> {
            self.pool_size = pool_size;
            Ok(42)
        }

        fn take_pixels(&mut self, _pool_fd: i32, pool_size: u32) -> Result<Vec<u8>, String> {
            assert_eq!(pool_size as usize, self.pixels.len());
            Ok(self.pixels.clone())
        }
    }

    #[test]
    fn wire_round_trip() {
        let mut buf = Vec::new();
        put_string(&mut buf, "wl_shm");
        let mut wrapped = vec![0u8; 4];
        wrapped.extend_from_slice(&buf);
        let mut at = 0;
        let _ = read_u32(&wrapped, &mut at);
        assert_eq!(read_string(&wrapped, &mut at).as_deref(), Some("wl_shm"));
        let req = frame_request(2, 1, &[9, 0, 0, 0]);
        let (events, rest) = parse_events(&req);
        assert!(rest.is_empty());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sender, 2);
    }

    #[test]
    fn full_capture_against_fake_compositor() {
        let mut fake = FakeCompositor::new_mode();
        let (w, h, pixels) = capture_over(&mut fake).expect("capture");
        assert_eq!((w, h), (64, 36));
        assert_eq!(pixels.len(), 64 * 36 * 4);
        // Pool sized for stride*height; shoot referenced output+buffer.
        assert_eq!(fake.pool_size, 64 * 36 * 4);
        assert!(!fake.inbox.is_empty());
        // The shoot request went to object 6 (shooter) opcode 0.
        let shoot = fake
            .inbox
            .iter()
            .find(|(b, _)| b.len() >= 8 && u32::from_le_bytes(b[0..4].try_into().unwrap()) == 6);
        assert!(shoot.is_some());
    }

    #[test]
    fn missing_screenshooter_is_honest_error() {
        let mut fake = FakeCompositor::new_mode();
        // Strip the shooter global from the first script chunk.
        let mut args = Vec::new();
        put_u32(&mut args, 10);
        put_string(&mut args, "wl_shm");
        put_u32(&mut args, 1);
        let mut args2 = Vec::new();
        put_u32(&mut args2, 11);
        put_string(&mut args2, "wl_output");
        put_u32(&mut args2, 3);
        let mut first = Vec::new();
        first.extend(FakeCompositor::event(2, 0, &args));
        first.extend(FakeCompositor::event(2, 0, &args2));
        first.extend(FakeCompositor::event(3, 0, &[]));
        fake.script[0] = first;
        let err = capture_over(&mut fake).unwrap_err();
        assert!(err.contains("weston_screenshooter"));
    }

    #[test]
    fn argb_shuffle_and_base64() {
        let mut px = vec![10u8, 20, 30, 255, 1, 2, 3, 4];
        argb_to_rgba(&mut px);
        assert_eq!(px, vec![30, 20, 10, 255, 3, 2, 1, 4]);
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn chunk_plan_caps() {
        assert_eq!(chunk_count(1).unwrap(), 1);
        assert_eq!(chunk_count(32 * 1024).unwrap(), 1);
        assert_eq!(chunk_count(32 * 1024 + 1).unwrap(), 2);
        assert!(chunk_count(0).is_err());
    }
}

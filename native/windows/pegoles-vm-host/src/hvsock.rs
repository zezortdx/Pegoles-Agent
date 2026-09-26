//! Hyper-V socket transport: host side of the guest control plane.
//!
//! Implements `pegoles_computer::GuestTransport` (same abstraction as the
//! macOS VirtioSocket transport): `send_frame / poll_events / close /
//! is_connected`, opaque JSONL frames, 64 KiB cap, reconnect-tolerant.
//! The Guest Protocol v1 above never knows which transport is active.
//!
//! - Windows host: `AF_HYPERV`, `SOCK_STREAM`, `HV_PROTOCOL_RAW`, bound to
//!   (VMID = HCS compute-system id, ServiceID = Pegoles service GUID).
//! - Linux guest: unchanged `AF_VSOCK` dial to CID 2, same port 4050.
//! - Off Windows this compiles to an honest stub (NotImplemented); the
//!   framing pump is platform-free and fully unit-tested via byte streams.
//!
//! NOTE: items below are live on Windows and in tests. The plain non-test
//! build on other OSes would otherwise flag them, so dead-code is allowed
//! exactly there; strictness is preserved on Windows and in all test builds.
#![cfg_attr(all(not(target_os = "windows"), not(test)), allow(dead_code))]

use pegoles_computer::{GuestTransport, TransportEvent};
use pegoles_guest_proto::MAX_FRAME_BYTES;
use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender};

/// Parse "00000FD2-facb-11e6-bd58-64006a7986d3" into raw GUID parts.
/// Single parser for service GUIDs (tested; never hand-roll elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl ServiceGuid {
    pub fn parse(s: &str) -> Option<Self> {
        let p: Vec<&str> = s.split('-').collect();
        if p.len() != 5 {
            return None;
        }
        let (a, b, c, d, e) = (p[0], p[1], p[2], p[3], p[4]);
        if a.len() != 8 || b.len() != 4 || c.len() != 4 || d.len() != 4 || e.len() != 12 {
            return None;
        }
        let data1 = u32::from_str_radix(a, 16).ok()?;
        let data2 = u16::from_str_radix(b, 16).ok()?;
        let data3 = u16::from_str_radix(c, 16).ok()?;
        let mut data4 = [0u8; 8];
        let tail = format!("{d}{e}");
        for (i, chunk) in tail.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).ok()?;
            data4[i] = u8::from_str_radix(s, 16).ok()?;
        }
        Some(Self {
            data1,
            data2,
            data3,
            data4,
        })
    }

    /// The Pegoles control-plane service (port 4050 → deterministic GUID).
    pub fn pegoles() -> Option<Self> {
        Self::parse(&pegoles_computer::hyperv_service_guid_for_port(
            pegoles_guest_proto::PEGOLES_VSOCK_PORT,
        ))
    }
}

/// Incremental byte-stream framer shared by the live pump and tests:
/// split on `\n` (strip one `\r`), enforce the 64 KiB cap, reject
/// non-UTF-8. Returns complete lines; keeps the tail buffered.
pub struct StreamFramer {
    pending: Vec<u8>,
}

impl StreamFramer {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, &'static str> {
        if self.pending.len() + bytes.len() > MAX_FRAME_BYTES + 4096 {
            self.pending.clear();
            return Err("frame_too_large");
        }
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
            line.pop(); // \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > MAX_FRAME_BYTES {
                self.pending.clear();
                return Err("frame_too_large");
            }
            match String::from_utf8(line) {
                Ok(s) => out.push(s),
                Err(_) => return Err("invalid_utf8"),
            }
        }
        if self.pending.len() > MAX_FRAME_BYTES {
            self.pending.clear();
            return Err("frame_too_large");
        }
        Ok(out)
    }
}

impl Default for StreamFramer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pump one connected byte stream into transport events. Pure I/O loop
/// over `Read`: the live socket on Windows, a `Cursor`/socketpair in
/// tests. Any violation or EOF ends the pump with a Disconnected event.
pub fn pump_stream(reader: &mut dyn Read, events: &Sender<TransportEvent>) -> TransportEvent {
    let mut framer = StreamFramer::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let ev = TransportEvent::Disconnected {
                    reason: "eof".to_string(),
                };
                let _ = events.send(ev.clone());
                return ev;
            }
            Ok(n) => match framer.push(&buf[..n]) {
                Ok(lines) => {
                    for line in lines {
                        let _ = events.send(TransportEvent::Frame(line));
                    }
                }
                Err(reason) => {
                    let ev = TransportEvent::Disconnected {
                        reason: reason.to_string(),
                    };
                    let _ = events.send(ev.clone());
                    return ev;
                }
            },
            Err(_) => {
                let ev = TransportEvent::Disconnected {
                    reason: "read_error".to_string(),
                };
                let _ = events.send(ev.clone());
                return ev;
            }
        }
    }
}

/// Frame writer: appends one newline-terminated frame to the live connection.
type FrameWriter = Box<dyn FnMut(&[u8]) -> std::io::Result<()> + Send>;

enum Link {
    Down,
    /// Live listener/connection. `events` carries pump output.
    Live {
        events: Receiver<TransportEvent>,
        writer: FrameWriter,
        stop: Sender<()>,
    },
}

/// Hyper-V socket transport for one computer. On Windows it owns the
/// AF_HYPERV listener + accept/pump threads; elsewhere it is an honest
/// stub (send fails closed, polls empty) so the session degrades to
/// Waiting → timeout Error instead of fantasy states.
pub struct HyperVSocketTransport {
    vmid: String,
    service_guid: String,
    link: Link,
    connected: bool,
}

impl HyperVSocketTransport {
    pub fn new(vmid: String) -> Self {
        Self {
            vmid,
            service_guid: pegoles_computer::hyperv_service_guid_for_port(
                pegoles_guest_proto::PEGOLES_VSOCK_PORT,
            ),
            link: Link::Down,
            connected: false,
        }
    }

    pub fn service_guid(&self) -> &str {
        &self.service_guid
    }

    /// HCS compute-system id this transport binds for (== Pegoles
    /// ComputerId; doubles as the AF_HYPERV VMID on Windows).
    pub fn vmid(&self) -> &str {
        &self.vmid
    }

    /// Start listening for the guest dial (idempotent). Real socket work
    /// happens on Windows; elsewhere records intent and stays Down.
    pub fn listen(&mut self) -> pegoles_computer::Result<()> {
        #[cfg(windows)]
        {
            self.listen_windows()
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(
                pegoles_computer::ComputerError::BackendFeatureNotImplemented(
                    "AF_HYPERV listener requires Windows".to_string(),
                ),
            )
        }
    }

    /// Test seam: attach an already-connected byte stream, bypassing real
    /// sockets. Used by unit tests on every OS. Emits Connected, then
    /// pumps the stream to EOF on a background thread like production.
    pub fn attach_test_stream(&mut self, reader: Box<dyn Read + Send>, writer: FrameWriter) {
        let (event_tx, event_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        // Connected first (channel order = event order), then pump.
        let _ = event_tx.send(TransportEvent::Connected);
        let pump_tx = event_tx;
        std::thread::spawn(move || {
            let mut reader = reader;
            pump_stream(&mut reader, &pump_tx);
            let _ = stop_rx.try_recv();
        });
        self.link = Link::Live {
            events: event_rx,
            writer,
            stop: stop_tx,
        };
        self.connected = true;
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub const AF_HYPERV: i32 = 34;
    const SOCK_STREAM: i32 = 1;
    const HV_PROTOCOL_RAW: i32 = 1;
    const INVALID_SOCKET: usize = !0usize;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Guid {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    #[repr(C)]
    struct HvSockAddr {
        family: u16,
        reserved: u16,
        vmid: Guid,
        service: Guid,
    }

    #[link(name = "ws2_32")]
    extern "system" {
        fn WSAStartup(wversion: u16, data: *mut u8) -> i32;
        fn socket(af: i32, typ: i32, protocol: i32) -> usize;
        fn bind(s: usize, name: *const HvSockAddr, namelen: i32) -> i32;
        fn listen(s: usize, backlog: i32) -> i32;
        fn accept(s: usize, addr: *mut HvSockAddr, addrlen: *mut i32) -> usize;
        fn send(s: usize, buf: *const u8, len: i32, flags: i32) -> i32;
        fn recv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
        fn closesocket(s: usize) -> i32;
        fn WSAGetLastError() -> i32;
    }

    pub fn to_guid(g: ServiceGuid) -> Guid {
        Guid {
            data1: g.data1,
            data2: g.data2,
            data3: g.data3,
            data4: g.data4,
        }
    }

    /// The computer UUID doubles as the HCS identity and the AF_HYPERV
    /// VMID (both are GUIDs by construction).
    pub fn parse_vmid(s: &str) -> Option<Guid> {
        ServiceGuid::parse(s).map(to_guid)
    }

    static WSA_ONCE: std::sync::Once = std::sync::Once::new();
    static mut WSA_OK: bool = false;

    /// Process-wide Winsock init (idempotent, thread-safe).
    pub fn wsa_startup() -> Result<(), String> {
        WSA_ONCE.call_once(|| {
            let mut data = [0u8; 512];
            unsafe {
                WSA_OK = WSAStartup(0x0202, data.as_mut_ptr()) == 0;
            }
        });
        if unsafe { WSA_OK } {
            Ok(())
        } else {
            Err("WSAStartup failed".to_string())
        }
    }

    pub fn last_error() -> i32 {
        unsafe { WSAGetLastError() }
    }

    pub fn raw_socket() -> Result<usize, String> {
        let sock = unsafe { socket(AF_HYPERV, SOCK_STREAM, HV_PROTOCOL_RAW) };
        if sock == INVALID_SOCKET {
            return Err(format!("AF_HYPERV socket failed: {}", last_error()));
        }
        Ok(sock)
    }

    pub fn raw_bind_listen(sock: usize, vmid: Guid, service: Guid) -> Result<(), String> {
        let addr = HvSockAddr {
            family: AF_HYPERV as u16,
            reserved: 0,
            vmid,
            service,
        };
        let ok = unsafe {
            bind(sock, &addr, std::mem::size_of::<HvSockAddr>() as i32) == 0
                && listen(sock, 16) == 0
        };
        if ok {
            Ok(())
        } else {
            Err(format!("AF_HYPERV bind/listen failed: {}", last_error()))
        }
    }

    pub fn raw_accept(sock: usize) -> Option<usize> {
        let mut peer = HvSockAddr {
            family: 0,
            reserved: 0,
            vmid: Guid {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0; 8],
            },
            service: Guid {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0; 8],
            },
        };
        let mut len = std::mem::size_of::<HvSockAddr>() as i32;
        let conn = unsafe { accept(sock, &mut peer, &mut len) };
        if conn == INVALID_SOCKET {
            None
        } else {
            Some(conn)
        }
    }

    pub fn raw_send_all(sock: usize, bytes: &[u8]) -> std::io::Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            let n = unsafe {
                send(
                    sock,
                    bytes[written..].as_ptr(),
                    (bytes.len() - written) as i32,
                    0,
                )
            };
            if n <= 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    format!("send failed: {}", last_error()),
                ));
            }
            written += n as usize;
        }
        Ok(())
    }

    pub fn raw_recv(sock: usize, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = unsafe { recv(sock, buf.as_mut_ptr(), buf.len() as i32, 0) };
        if n < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                format!("recv failed: {}", last_error()),
            ));
        }
        Ok(n as usize)
    }

    pub fn raw_close(sock: usize) {
        unsafe {
            closesocket(sock);
        }
    }

    impl HyperVSocketTransport {
        pub(crate) fn listen_windows(&mut self) -> pegoles_computer::Result<()> {
            use pegoles_computer::ComputerError;
            wsa_startup().map_err(ComputerError::Backend)?;
            let service = ServiceGuid::parse(&self.service_guid)
                .ok_or_else(|| ComputerError::Backend("invalid service GUID".to_string()))?;
            let vmid = parse_vmid(&self.vmid)
                .ok_or_else(|| ComputerError::Backend("computer id is not a GUID".to_string()))?;
            let sock = raw_socket().map_err(ComputerError::Backend)?;
            if let Err(e) = raw_bind_listen(sock, to_guid(vmid), to_guid(service)) {
                raw_close(sock);
                return Err(ComputerError::Backend(e));
            }
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
            // Current accepted connection, shared with the writer closure.
            let current: std::sync::Arc<std::sync::Mutex<Option<usize>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            let current_accept = current.clone();
            std::thread::spawn(move || {
                loop {
                    if stop_rx.try_recv().is_ok() {
                        raw_close(sock);
                        return;
                    }
                    let Some(conn) = raw_accept(sock) else {
                        if stop_rx.try_recv().is_ok() {
                            raw_close(sock);
                            return;
                        }
                        continue;
                    };
                    // A fresh dial replaces the old link (guest restart),
                    // mirroring the macOS helper.
                    if let Some(old) = current_accept.lock().expect("conn").replace(conn) {
                        raw_close(old);
                    }
                    let _ = event_tx.send(TransportEvent::Connected);
                    let pump_tx = event_tx.clone();
                    let mut buf = [0u8; 4096];
                    let mut framer = StreamFramer::new();
                    loop {
                        match raw_recv(conn, &mut buf) {
                            Ok(0) => {
                                let _ = pump_tx.send(TransportEvent::Disconnected {
                                    reason: "eof".to_string(),
                                });
                                break;
                            }
                            Ok(n) => match framer.push(&buf[..n]) {
                                Ok(lines) => {
                                    for line in lines {
                                        let _ = pump_tx.send(TransportEvent::Frame(line));
                                    }
                                }
                                Err(reason) => {
                                    let _ = pump_tx.send(TransportEvent::Disconnected {
                                        reason: reason.to_string(),
                                    });
                                    break;
                                }
                            },
                            Err(_) => {
                                let _ = pump_tx.send(TransportEvent::Disconnected {
                                    reason: "read_error".to_string(),
                                });
                                break;
                            }
                        }
                    }
                    raw_close(conn);
                }
            });
            let writer = move |bytes: &[u8]| -> std::io::Result<()> {
                let guard = current.lock().expect("conn");
                match *guard {
                    Some(conn) => raw_send_all(conn, bytes),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        "no guest connection yet",
                    )),
                }
            };
            self.link = Link::Live {
                events: event_rx,
                writer: Box::new(writer),
                stop: stop_tx,
            };
            self.connected = false;
            Ok(())
        }
    }
}

impl GuestTransport for HyperVSocketTransport {
    fn send_frame(&mut self, payload: &str) -> Result<(), pegoles_computer::ComputerError> {
        use pegoles_computer::ComputerError;
        if payload.contains('\n') {
            return Err(ComputerError::Backend(
                "guest payload must be one frame".to_string(),
            ));
        }
        if payload.len() > MAX_FRAME_BYTES {
            return Err(ComputerError::Backend("guest frame too large".to_string()));
        }
        match &mut self.link {
            Link::Live { writer, .. } => {
                let mut line = payload.as_bytes().to_vec();
                line.push(b'\n');
                writer(&line).map_err(|e| ComputerError::GuestUnavailable(e.to_string()))?;
                Ok(())
            }
            Link::Down => Err(ComputerError::GuestUnavailable(
                "no guest connection".to_string(),
            )),
        }
    }

    fn poll_events(&mut self) -> Vec<TransportEvent> {
        match &mut self.link {
            Link::Live { events, .. } => events.try_iter().collect(),
            Link::Down => Vec::new(),
        }
    }

    fn close(&mut self) {
        if let Link::Live { stop, .. } = &self.link {
            let _ = stop.send(());
        }
        self.link = Link::Down;
        self.connected = false;
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;

    #[test]
    fn service_guid_parses_and_matches_port_derivation() {
        let g = ServiceGuid::parse("00000FD2-facb-11e6-bd58-64006a7986d3").unwrap();
        assert_eq!(g.data1, 4050);
        assert_eq!(g.data2, 0xfacb);
        assert_eq!(
            ServiceGuid::pegoles().unwrap(),
            ServiceGuid::parse(&pegoles_computer::hyperv_service_guid_for_port(4050)).unwrap()
        );
        assert!(ServiceGuid::parse("not-a-guid").is_none());
        assert!(ServiceGuid::parse("00000FD2-facb-11e6-bd58-64006a7986d").is_none());
    }

    #[test]
    fn framer_splits_partials_batches_and_caps() {
        let mut f = StreamFramer::new();
        assert!(f.push(b"{\"a\":").unwrap().is_empty());
        assert_eq!(
            f.push(b"1}\n{\"b\":2}\n").unwrap(),
            vec!["{\"a\":1}", "{\"b\":2}"]
        );
        let mut f = StreamFramer::new();
        assert_eq!(f.push(b"x\r\ny\n").unwrap(), vec!["x", "y"]);
        let mut f = StreamFramer::new();
        let big = vec![b'z'; MAX_FRAME_BYTES + 1];
        assert!(f.push(&big).is_err());
        let mut f = StreamFramer::new();
        assert!(f.push(b"\xff\xfe\n").is_err());
    }

    #[test]
    fn pump_survives_eof_mid_frame() {
        let (tx, rx) = mpsc::channel();
        let mut cur = Cursor::new(b"{\"half\":".to_vec());
        let terminal = pump_stream(&mut cur, &tx);
        assert!(matches!(terminal, TransportEvent::Disconnected { .. }));
        // The partial frame never surfaces; only the disconnect notice does.
        let evs: Vec<_> = rx.try_iter().collect();
        assert!(evs
            .iter()
            .all(|e| matches!(e, TransportEvent::Disconnected { .. })));
        assert!(!evs.is_empty());
    }

    #[test]
    fn transport_test_stream_round_trip() {
        let written = std::sync::Arc::new(Mutex::new(Vec::<u8>::new()));
        let written2 = written.clone();
        let mut t = HyperVSocketTransport::new("computer-1".to_string());
        assert_eq!(t.vmid(), "computer-1");
        assert!(t.service_guid().ends_with("facb-11e6-bd58-64006a7986d3"));
        t.attach_test_stream(
            Box::new(Cursor::new(
                b"{\"type\":\"guest_hello\",\"protocol_version\":1}\n".to_vec(),
            )),
            Box::new(move |bytes: &[u8]| {
                written2.lock().expect("w").extend_from_slice(bytes);
                Ok(())
            }),
        );
        assert!(t.is_connected());
        t.send_frame(r#"{"type":"host_hello"}"#).unwrap();
        // Pump thread may need a moment; poll until the frame surfaces.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            for ev in t.poll_events() {
                if matches!(ev, TransportEvent::Frame(_)) {
                    found = true;
                }
            }
            if found {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(found, "frame from test stream never surfaced");
        let out = written.lock().expect("w").clone();
        assert_eq!(out, b"{\"type\":\"host_hello\"}\n");
        t.close();
        assert!(!t.is_connected());
        assert!(t.send_frame("{}").is_err());
    }

    #[test]
    fn session_runs_above_hyperv_shaped_transport() {
        // Same GuestSession, HyperV-shaped transport: protocol independence.
        use pegoles_computer::{GuestObservation, GuestSession};
        use pegoles_protocol::GuestRuntimeState as G;
        let mut t = HyperVSocketTransport::new("computer-9".to_string());
        t.attach_test_stream(
            Box::new(Cursor::new(
                concat!(
                    "{\"type\":\"guest_hello\",\"protocol_version\":1,",
                    "\"runtime_version\":\"0.1.0\",\"os\":\"debian\",",
                    "\"os_version\":\"13\",\"arch\":\"x86_64\"}\n",
                    "{\"type\":\"ready\"}\n",
                )
                .as_bytes()
                .to_vec(),
            )),
            Box::new(|_: &[u8]| Ok(())),
        );
        let mut s = GuestSession::new();
        let now = std::time::Instant::now();
        s.on_vm_started(now);
        // Connected arrives first (attach emits it), then the two frames.
        // Break at BecameReady: the Cursor EOF that follows would surface
        // a Disconnected right after, which is correct transport behavior
        // but not what this handshake assertion is about.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut ready = false;
        'pump: while std::time::Instant::now() < deadline && !ready {
            for ev in t.poll_events() {
                let outcomes = match ev {
                    TransportEvent::Connected => s.on_connected(now),
                    TransportEvent::Frame(line) => s.on_frame(&line, now),
                    TransportEvent::Disconnected { reason } => s.on_disconnected(reason),
                };
                for o in outcomes {
                    let (action, observation) = o.split();
                    if let Some(pegoles_computer::Outbound::Send(f)) = action {
                        t.send_frame(&f).unwrap();
                    }
                    if let Some(GuestObservation::BecameReady { .. }) = observation {
                        ready = true;
                        break 'pump;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready, "handshake never completed over test stream");
        assert_eq!(s.state(), G::Ready);
    }
}

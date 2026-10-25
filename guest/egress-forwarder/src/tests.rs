//! Session logic over a Unix socketpair (fake host) and real loopback TCP.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pegoles_egress_proto::{
    Conn, Decoder, Frame, Role, INITIAL_WINDOW, MAX_PAYLOAD, MAX_STREAMS, VERSION,
};

use crate::policy_file::{policy_json, EMPTY_POLICY};
use crate::session::run_session;
use crate::{server, Config};

const WAIT: Duration = Duration::from_secs(5);

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique(ext: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("pgf-{}-{n}.{ext}", std::process::id()))
}

/// The managed-policy file, pre-existing and empty like in the image.
struct PolicyFile(PathBuf);

impl PolicyFile {
    fn new() -> PolicyFile {
        let p = unique("json");
        std::fs::write(&p, EMPTY_POLICY).unwrap();
        PolicyFile(p)
    }
    fn contents(&self) -> String {
        std::fs::read_to_string(&self.0).unwrap()
    }
}

impl Drop for PolicyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn ca_der() -> Vec<u8> {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let key = rcgen::KeyPair::generate().unwrap();
    params.self_signed(&key).unwrap().der().to_vec()
}

fn config(policy: &PolicyFile) -> (Config, Receiver<SocketAddr>) {
    let (tx, rx) = mpsc::channel();
    let cfg = Config {
        policy_file: policy.0.clone(),
        proxy_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        listening: Some(tx),
    };
    (cfg, rx)
}

/// The host end: speaks the mux protocol by hand.
struct Host {
    io: UnixStream,
    dec: Decoder,
    conn: Conn,
    pending: std::collections::VecDeque<Frame>,
}

impl Host {
    fn new(io: UnixStream) -> Host {
        io.set_read_timeout(Some(WAIT)).unwrap();
        Host {
            io,
            dec: Decoder::new(),
            conn: Conn::new(Role::Host),
            pending: Default::default(),
        }
    }

    fn send(&mut self, f: &Frame) {
        let mut buf = Vec::new();
        f.encode(&mut buf).unwrap();
        self.io.write_all(&buf).expect("host write");
    }

    fn hello(&mut self, ca: Vec<u8>) {
        let f = self.conn.hello_frame(ca).unwrap();
        self.send(&f);
    }

    /// Next frame, or `None` on EOF / timeout.
    fn recv_within(&mut self, timeout: Duration) -> Option<Frame> {
        // macOS rejects setsockopt on a socket the peer already closed.
        let _ = self.io.set_read_timeout(Some(timeout));
        loop {
            if let Some(f) = self.pending.pop_front() {
                return Some(f);
            }
            let mut buf = [0u8; 65536];
            let n = self.io.read(&mut buf).ok().filter(|&n| n > 0)?;
            let mut frames = Vec::new();
            self.dec.decode_all(&buf[..n], &mut frames).unwrap();
            self.pending.extend(frames);
        }
    }

    fn recv(&mut self) -> Frame {
        self.recv_within(WAIT).expect("frame from the forwarder")
    }
}

struct Running {
    host: Host,
    policy: PolicyFile,
    listening: Receiver<SocketAddr>,
    thread: JoinHandle<()>,
}

fn start() -> Running {
    let policy = PolicyFile::new();
    let (cfg, listening) = config(&policy);
    let (a, b) = UnixStream::pair().unwrap();
    let thread = thread::spawn(move || run_session(b, &cfg));
    Running {
        host: Host::new(a),
        policy,
        listening,
        thread,
    }
}

/// Handshake done; returns the proxy address too.
fn start_ready() -> (Running, SocketAddr) {
    let mut r = start();
    r.host.hello(ca_der());
    assert_eq!(r.host.recv(), Frame::HelloAck { version: VERSION });
    let addr = r.listening.recv_timeout(WAIT).expect("proxy bound");
    (r, addr)
}

fn client(addr: SocketAddr) -> TcpStream {
    let c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(WAIT)).unwrap();
    c
}

fn read_exact_n(c: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut out = vec![0; n];
    c.read_exact(&mut out).unwrap();
    out
}

fn is_eof(c: &mut TcpStream) -> bool {
    let mut b = [0u8; 1];
    matches!(c.read(&mut b), Ok(0) | Err(_))
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn handshake_installs_the_ca_then_acks_then_listens() {
    let mut r = start();
    // nothing listens before the handshake
    assert!(r.listening.try_recv().is_err());
    let ca = ca_der();
    r.host.hello(ca.clone());
    assert_eq!(r.host.recv(), Frame::HelloAck { version: VERSION });
    // the policy file was complete before the ACK arrived
    assert_eq!(r.policy.contents(), policy_json(&ca));
    let addr = r.listening.recv_timeout(WAIT).unwrap();
    assert!(addr.ip().is_loopback());
    drop(client(addr));
}

#[test]
fn only_the_host_cid_may_connect() {
    use crate::transport::{is_host_cid, VMADDR_CID_HOST};
    assert_eq!(VMADDR_CID_HOST, 2);
    assert!(is_host_cid(2));
    for cid in [0, 1, 3, 4, 100, u32::MAX - 1, u32::MAX] {
        assert!(!is_host_cid(cid), "cid {cid}");
    }
}

#[test]
fn a_failed_proxy_bind_ends_the_session_without_a_hello_ack() {
    let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let policy = PolicyFile::new();
    let (mut cfg, listening) = config(&policy);
    cfg.proxy_addr = taken.local_addr().unwrap();
    let (a, b) = UnixStream::pair().unwrap();
    let thread = thread::spawn(move || run_session(b, &cfg));
    let mut host = Host::new(a);
    host.hello(ca_der());
    // The host must never be told the channel is ready.
    assert_eq!(host.recv_within(WAIT), None);
    thread.join().unwrap();
    assert!(listening.try_recv().is_err());
    assert_eq!(policy.contents(), EMPTY_POLICY);
}

#[test]
fn a_bad_hello_drops_the_channel_and_leaves_the_policy_empty() {
    let mut non_ca_params = rcgen::CertificateParams::new(vec!["x.test".to_string()]).unwrap();
    non_ca_params.is_ca = rcgen::IsCa::NoCa;
    let non_ca = non_ca_params
        .self_signed(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec();
    let mut truncated = ca_der();
    truncated.truncate(truncated.len() - 1);
    let cases = [
        Frame::Hello {
            version: VERSION,
            ca_der: b"not a certificate".to_vec(),
        },
        Frame::Hello {
            version: VERSION,
            ca_der: non_ca,
        },
        Frame::Hello {
            version: VERSION,
            ca_der: truncated,
        },
        Frame::Hello {
            version: VERSION + 1,
            ca_der: ca_der(),
        },
        Frame::Open { stream: 1 },
    ];
    for bad in cases {
        let mut r = start();
        r.host.send(&bad);
        assert!(r.host.recv_within(WAIT).is_none(), "{bad:?}: no ACK, EOF");
        r.thread.join().unwrap();
        assert_eq!(r.policy.contents(), EMPTY_POLICY, "{bad:?}");
        assert!(r.listening.try_recv().is_err(), "{bad:?}");
    }
}

#[test]
fn data_round_trips_and_credit_is_granted_as_tcp_drains() {
    let (mut r, addr) = start_ready();
    let mut c = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 1 });

    c.write_all(b"hello").unwrap();
    assert_eq!(
        r.host.recv(),
        Frame::Data {
            stream: 1,
            payload: b"hello".to_vec()
        }
    );

    // host to client: three full frames, credit comes back as they drain
    for _ in 0..3 {
        r.host.send(&Frame::Data {
            stream: 1,
            payload: vec![7; MAX_PAYLOAD],
        });
    }
    assert!(read_exact_n(&mut c, 3 * MAX_PAYLOAD)
        .iter()
        .all(|&b| b == 7));
    let mut granted = 0u64;
    while granted < (3 * MAX_PAYLOAD) as u64 {
        match r.host.recv() {
            Frame::Credit {
                stream: 1,
                increment,
            } => granted += u64::from(increment),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(granted, (3 * MAX_PAYLOAD) as u64);
}

#[test]
fn the_forwarder_never_sends_beyond_the_window() {
    let (mut r, addr) = start_ready();
    let c = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 1 });
    let total = INITIAL_WINDOW as usize + 40_000;
    let mut w = c.try_clone().unwrap();
    let writer = thread::spawn(move || w.write_all(&vec![1u8; total]).unwrap());

    let mut got = 0usize;
    while got < INITIAL_WINDOW as usize {
        match r.host.recv() {
            Frame::Data { stream: 1, payload } => got += payload.len(),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(got, INITIAL_WINDOW as usize);
    // out of credit: nothing more may arrive
    assert!(r.host.recv_within(Duration::from_millis(300)).is_none());
    // the silent period above timed out the read; the channel is still up
    r.host.send(&Frame::Credit {
        stream: 1,
        increment: 100_000,
    });
    while got < total {
        match r.host.recv() {
            Frame::Data { stream: 1, payload } => got += payload.len(),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(got, total);
    writer.join().unwrap();
}

#[test]
fn a_host_close_flushes_queued_data_then_closes_the_client() {
    let (mut r, addr) = start_ready();
    let mut c = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 1 });
    r.host.send(&Frame::Data {
        stream: 1,
        payload: b"bye".to_vec(),
    });
    r.host.send(&Frame::Close { stream: 1 });
    let mut all = Vec::new();
    c.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"bye");
    // the forwarder does not answer a CLOSE with a CLOSE
    assert!(r.host.recv_within(Duration::from_millis(200)).is_none());
}

#[test]
fn a_client_disconnect_sends_close() {
    let (mut r, addr) = start_ready();
    let c = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 1 });
    drop(c);
    assert_eq!(r.host.recv(), Frame::Close { stream: 1 });
    // a later connection gets the next odd id
    let _c2 = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 3 });
}

#[test]
fn channel_close_resets_the_policy_and_closes_everything() {
    let (mut r, addr) = start_ready();
    let mut c1 = client(addr);
    let mut c2 = client(addr);
    assert_eq!(r.host.recv(), Frame::Open { stream: 1 });
    assert_eq!(r.host.recv(), Frame::Open { stream: 3 });
    assert_ne!(r.policy.contents(), EMPTY_POLICY);

    let Running {
        host,
        policy,
        thread,
        ..
    } = r;
    drop(host);
    thread.join().unwrap();

    assert_eq!(policy.contents(), EMPTY_POLICY);
    assert!(is_eof(&mut c1));
    assert!(is_eof(&mut c2));
    // the listener is gone
    assert!(TcpStream::connect(addr).is_err());
}

#[test]
fn a_protocol_violation_ends_the_session() {
    let (mut r, _addr) = start_ready();
    // DATA for a stream that was never opened
    r.host.send(&Frame::Data {
        stream: 9,
        payload: vec![1],
    });
    assert!(r.host.recv_within(WAIT).is_none());
    r.thread.join().unwrap();
    assert_eq!(r.policy.contents(), EMPTY_POLICY);
}

#[test]
fn connections_beyond_the_stream_limit_are_refused() {
    let (mut r, addr) = start_ready();
    let mut held = Vec::new();
    for i in 0..MAX_STREAMS {
        held.push(client(addr));
        let id = 2 * i as u32 + 1;
        assert_eq!(r.host.recv(), Frame::Open { stream: id });
    }
    let mut extra = client(addr);
    assert!(is_eof(&mut extra));
    assert!(r.host.recv_within(Duration::from_millis(200)).is_none());
    // a freed slot is reusable
    drop(held.pop());
    assert!(matches!(r.host.recv(), Frame::Close { .. }));
    let _again = client(addr);
    assert!(matches!(r.host.recv(), Frame::Open { .. }));
}

fn serve_on(policy: &PolicyFile) -> (PathBuf, Receiver<SocketAddr>) {
    let sock = unique("sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (cfg, rx) = config(policy);
    thread::spawn(move || server::serve(&listener, &cfg));
    (sock, rx)
}

#[test]
fn a_second_host_connection_is_refused_while_one_is_active() {
    let policy = PolicyFile::new();
    let (sock, rx) = serve_on(&policy);

    let mut first = Host::new(UnixStream::connect(&sock).unwrap());
    first.hello(ca_der());
    assert_eq!(first.recv(), Frame::HelloAck { version: VERSION });
    rx.recv_timeout(WAIT).unwrap();

    let mut second = Host::new(UnixStream::connect(&sock).unwrap());
    assert!(second.recv_within(WAIT).is_none(), "refused: EOF");
    // the first is unaffected and still owns the policy
    assert_ne!(policy.contents(), EMPTY_POLICY);

    // once the first ends, the next host is served
    drop(first);
    wait_until("policy reset", || policy.contents() == EMPTY_POLICY);
    let end = Instant::now() + WAIT;
    loop {
        let mut next = Host::new(UnixStream::connect(&sock).unwrap());
        // refused (EPIPE) while the previous session is still winding down
        let hello = next.conn.hello_frame(ca_der()).unwrap();
        let mut bytes = Vec::new();
        hello.encode(&mut bytes).unwrap();
        let _ = next.io.write_all(&bytes);
        if next.recv_within(Duration::from_millis(500)).is_some() {
            break;
        }
        assert!(Instant::now() < end, "a new host was never served");
    }
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn startup_resets_a_stale_policy_file() {
    let policy = PolicyFile::new();
    std::fs::write(&policy.0, policy_json(&ca_der())).unwrap();
    let (sock, _rx) = serve_on(&policy);
    wait_until("startup reset", || policy.contents() == EMPTY_POLICY);
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn policy_file_is_written_in_place_and_never_created() {
    let missing: &Path = &unique("json");
    assert!(crate::policy_file::install(missing, &ca_der()).is_err());
    assert!(!missing.exists());
}

// ---- real host (pegoles-egress) over the socketpair ---------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_host_session_denies_a_host_outside_the_allowlist() {
    use pegoles_egress::{audit, EgressSession, Mode};

    let policy = PolicyFile::new();
    let (cfg, listening) = config(&policy);
    let (host_end, guest_end) = UnixStream::pair().unwrap();
    host_end.set_nonblocking(true).unwrap();
    let forwarder = thread::spawn(move || run_session(guest_end, &cfg));

    let (sink, _events) = audit::channel();
    let session = EgressSession::start(
        tokio::net::UnixStream::from_std(host_end).unwrap(),
        Mode::allowlist(["allowed.example"]).unwrap(),
        sink,
    )
    .await
    .expect("session starts");
    let addr = listening.recv_timeout(WAIT).unwrap();
    assert!(policy.contents().contains("CACertificates"));
    assert_ne!(policy.contents(), EMPTY_POLICY);

    let reply = tokio::task::spawn_blocking(move || {
        let mut c = client(addr);
        c.write_all(b"CONNECT denied.example:443 HTTP/1.1\r\nHost: denied.example:443\r\n\r\n")
            .unwrap();
        let mut out = Vec::new();
        let _ = c.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    })
    .await
    .unwrap();
    assert!(reply.starts_with("HTTP/1.1 403"), "got {reply:?}");

    session.shutdown().await;
    tokio::task::spawn_blocking(move || forwarder.join().unwrap())
        .await
        .unwrap();
    assert_eq!(policy.contents(), EMPTY_POLICY);
}

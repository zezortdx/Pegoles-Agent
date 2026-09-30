use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pegoles_egress_proto::{Frame, ProtoError, MAX_STREAMS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::testutil::FakeGuest;

fn echo_handler() -> Handler {
    Arc::new(|mut s: MuxStream| {
        Box::pin(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let n = s.read(&mut buf).await.unwrap_or(0);
                if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            let _ = s.shutdown().await;
        })
    })
}

async fn setup(handler: Handler) -> (Running, FakeGuest) {
    let (host_io, guest_io) = tokio::io::duplex(64 * 1024);
    let (running, guest) = tokio::join!(
        start_host(host_io, vec![0x30, 1, 2, 3], handler),
        FakeGuest::connect(guest_io)
    );
    (running.expect("handshake"), guest)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

#[tokio::test]
async fn hello_carries_the_ca_and_streams_echo_past_the_window() {
    let (_running, guest) = setup(echo_handler()).await;
    assert_eq!(guest.ca_der, vec![0x30, 1, 2, 3]);
    let stream = guest.open();
    let (mut rd, mut wr) = tokio::io::split(stream);
    let data = pattern(1024 * 1024); // four windows
    let to_send = data.clone();
    let writer = tokio::spawn(async move {
        wr.write_all(&to_send).await.expect("write");
        wr
    });
    let mut back = vec![0u8; data.len()];
    tokio::time::timeout(Duration::from_secs(20), rd.read_exact(&mut back))
        .await
        .expect("no stall")
        .expect("read");
    assert_eq!(back, data);
    drop(writer.await.expect("writer"));
}

#[tokio::test]
async fn many_streams_run_concurrently() {
    let (_running, guest) = setup(echo_handler()).await;
    let mut tasks = Vec::new();
    for i in 0..16usize {
        let stream = guest.open();
        tasks.push(tokio::spawn(async move {
            let (mut rd, mut wr) = tokio::io::split(stream);
            let data = pattern(100_000 + i);
            let d2 = data.clone();
            let w = tokio::spawn(async move {
                wr.write_all(&d2).await.expect("write");
                wr
            });
            let mut back = vec![0u8; data.len()];
            rd.read_exact(&mut back).await.expect("read");
            assert_eq!(back, data);
            drop(w.await.expect("writer"));
        }));
    }
    for t in tasks {
        tokio::time::timeout(Duration::from_secs(30), t)
            .await
            .expect("no stall")
            .expect("task");
    }
}

#[tokio::test]
async fn writes_stall_at_the_window_when_the_peer_does_not_read() {
    let sleeper: Handler = Arc::new(|s: MuxStream| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(s);
        })
    });
    let (_running, guest) = setup(sleeper).await;
    let mut stream = guest.open();
    let too_much = vec![1u8; pegoles_egress_proto::INITIAL_WINDOW as usize + 50_000];
    let r = tokio::time::timeout(Duration::from_millis(400), stream.write_all(&too_much)).await;
    assert!(
        r.is_err(),
        "the sender must wait for credit beyond the initial window"
    );
}

struct SetOnDrop(Arc<AtomicBool>);
impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn dropping_the_session_kills_every_stream_task() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let handler: Handler = Arc::new(move |mut s: MuxStream| {
        let guard = SetOnDrop(f.clone());
        Box::pin(async move {
            let _guard = guard;
            let mut b = [0u8; 16];
            while s.read(&mut b).await.unwrap_or(0) > 0 {}
            tokio::time::sleep(Duration::from_secs(60)).await;
        })
    });
    let (running, guest) = setup(handler).await;
    let mut stream = guest.open();
    stream.write_all(b"x").await.expect("write");
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(running);
    let mut b = [0u8; 4];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut b))
        .await
        .expect("guest notices the closed channel")
        .expect("read");
    assert_eq!(n, 0);
    for _ in 0..50 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("stream handler task was not aborted");
}

#[tokio::test]
async fn shutdown_ends_the_session_and_reports_it() {
    let (running, _guest) = setup(echo_handler()).await;
    assert!(!running.is_closed());
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        running.shutdown().await;
    })
    .await;
    assert!(closed.is_ok());
}

#[tokio::test]
async fn frame_for_an_unknown_stream_drops_the_connection() {
    let (running, guest) = setup(echo_handler()).await;
    let bad = Frame::Data {
        stream: 99,
        payload: vec![1],
    };
    guest.core.send_raw(bad.to_bytes().expect("encode"));
    let end = tokio::time::timeout(Duration::from_secs(5), running.closed())
        .await
        .expect("session ends");
    assert_eq!(end, SessionEnd::Protocol(ProtoError::UnknownStream));
    assert!(running.is_closed());
}

#[tokio::test]
async fn garbage_and_oversize_frames_drop_the_connection() {
    for raw in [
        vec![0u8, 0, 0, 1, 9, 0, 0, 0, 0],             // unknown kind
        vec![0u8, 0, 0, 1, 3, 0xff, 0xff, 0xff, 0xff], // absurd length
        vec![0u8, 0, 0, 1, 3, 0, 0, 0x40, 0x01],       // 16385
    ] {
        let (running, guest) = setup(echo_handler()).await;
        guest.core.send_raw(raw);
        let end = tokio::time::timeout(Duration::from_secs(5), running.closed())
            .await
            .expect("session ends");
        assert!(matches!(end, SessionEnd::Protocol(_)), "{end:?}");
    }
}

#[tokio::test]
async fn the_65th_open_stream_drops_the_connection() {
    let sleeper: Handler = Arc::new(|s: MuxStream| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(s);
        })
    });
    let (running, guest) = setup(sleeper).await;
    let _streams: Vec<_> = (0..MAX_STREAMS).map(|_| guest.open()).collect();
    assert!(guest.core.open_stream().is_err(), "the guest refuses too");
    let open = Frame::Open {
        stream: 2 * MAX_STREAMS as u32 + 1,
    };
    guest.core.send_raw(open.to_bytes().expect("encode"));
    let end = tokio::time::timeout(Duration::from_secs(5), running.closed())
        .await
        .expect("session ends");
    assert_eq!(end, SessionEnd::Protocol(ProtoError::TooManyStreams));
}

#[tokio::test]
async fn the_guest_going_away_is_reported() {
    let (running, guest) = setup(echo_handler()).await;
    drop(guest);
    let end = tokio::time::timeout(Duration::from_secs(5), running.closed())
        .await
        .expect("session ends");
    assert_eq!(end, SessionEnd::PeerClosed);
}

#[tokio::test]
async fn a_guest_that_speaks_before_hello_ack_fails_the_handshake() {
    let (host_io, mut guest_io) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        start_host(host_io, vec![1, 2, 3], echo_handler())
            .await
            .err()
    });
    // read HELLO, then answer with an OPEN instead of HELLO_ACK
    let mut buf = [0u8; 14]; // 9-byte header + version + 3-byte CA
    guest_io.read_exact(&mut buf).await.expect("HELLO");
    let open = Frame::Open { stream: 1 }.to_bytes().expect("encode");
    guest_io.write_all(&open).await.expect("write");
    let err = task.await.expect("join").expect("handshake must fail");
    assert!(
        matches!(err, HandshakeError::Protocol(ProtoError::OutOfOrder)),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_wrong_protocol_version_fails_the_handshake() {
    let (host_io, mut guest_io) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        start_host(host_io, vec![1, 2, 3], echo_handler())
            .await
            .err()
    });
    let mut buf = [0u8; 14]; // 9-byte header + version + 3-byte CA
    guest_io.read_exact(&mut buf).await.expect("HELLO");
    let ack = Frame::HelloAck { version: 7 }.to_bytes().expect("encode");
    guest_io.write_all(&ack).await.expect("write");
    let err = task.await.expect("join").expect("handshake must fail");
    assert!(
        matches!(
            err,
            HandshakeError::Protocol(ProtoError::UnsupportedVersion(7))
        ),
        "{err:?}"
    );
}

/// Counts handlers from the moment the handler closure runs until its
/// future is dropped (finished, aborted or never polled).
struct Live {
    started: Arc<std::sync::atomic::AtomicUsize>,
    now: Arc<std::sync::atomic::AtomicUsize>,
    max: Arc<std::sync::atomic::AtomicUsize>,
}

struct LiveGuard(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn live_handler() -> (Handler, Live) {
    let now = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (n, m, st) = (now.clone(), max.clone(), started.clone());
    let handler: Handler = Arc::new(move |s: MuxStream| {
        st.fetch_add(1, Ordering::SeqCst);
        let cur = n.fetch_add(1, Ordering::SeqCst) + 1;
        m.fetch_max(cur, Ordering::SeqCst);
        let guard = LiveGuard(n.clone());
        Box::pin(async move {
            let _guard = guard;
            let _s = s;
            // Like a connect to a blackhole: lives until aborted.
            tokio::time::sleep(Duration::from_secs(300)).await;
        })
    });
    (handler, Live { started, now, max })
}

async fn wait_live_zero(live: &Live) {
    for _ in 0..100 {
        if live.now.load(Ordering::SeqCst) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "handlers were not torn down: {}",
        live.now.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn a_peer_close_aborts_the_stream_handler() {
    let (handler, live) = live_handler();
    let (_running, guest) = setup(handler).await;
    let stream = guest.open();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(live.now.load(Ordering::SeqCst), 1);
    drop(stream); // CLOSE
    wait_live_zero(&live).await;
}

#[tokio::test]
async fn open_request_close_loops_cannot_exceed_64_live_handlers() {
    let (handler, live) = live_handler();
    let (running, guest) = setup(handler).await;
    // 1) paced: each stream is closed while its handler is stuck.
    for _ in 0..200 {
        let mut s = guest.open();
        s.write_all(b"GET http://blackhole/ HTTP/1.1\r\n\r\n")
            .await
            .expect("write");
        drop(s);
    }
    // 2) one burst of OPEN,CLOSE pairs that the host decodes without
    // yielding: aborted tasks have not been dropped yet.
    let mut burst = Vec::new();
    for i in 0..300u32 {
        let id = 1001 + 2 * i;
        burst.extend(Frame::Open { stream: id }.to_bytes().expect("encode"));
        burst.extend(Frame::Close { stream: id }.to_bytes().expect("encode"));
    }
    // The last stream stays open: once its handler runs, everything before
    // it has been processed (the host handles frames in order).
    burst.extend(Frame::Open { stream: 3001 }.to_bytes().expect("encode"));
    guest.core.send_raw(burst);
    for _ in 0..100 {
        if live.started.load(Ordering::SeqCst) > 0 && live.now.load(Ordering::SeqCst) >= 1 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    guest
        .core
        .send_raw(Frame::Close { stream: 3001 }.to_bytes().expect("encode"));
    wait_live_zero(&live).await;
    assert!(
        live.started.load(Ordering::SeqCst) >= 100,
        "the loop must have run"
    );
    assert!(
        live.max.load(Ordering::SeqCst) <= MAX_STREAMS,
        "peak live handlers {}",
        live.max.load(Ordering::SeqCst)
    );
    assert!(
        !running.is_closed(),
        "refusing streams must not kill the session"
    );
}

#[tokio::test]
async fn one_byte_frames_do_not_multiply_queued_memory() {
    let held: Arc<Mutex<Option<MuxStream>>> = Arc::new(Mutex::new(None));
    let h2 = held.clone();
    let handler: Handler = Arc::new(move |s: MuxStream| {
        *h2.lock().expect("lock") = Some(s);
        Box::pin(async move { tokio::time::sleep(Duration::from_secs(300)).await })
    });
    let (_running, guest) = setup(handler).await;
    let mut stream = guest.open();
    let window = pegoles_egress_proto::INITIAL_WINDOW as usize;
    for _ in 0..window {
        stream.write_all(&[7]).await.expect("write");
    }
    // Wait for the host to have queued all of it.
    let mut stats = (0, 0);
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(s) = held.lock().expect("lock").as_ref() {
            stats = s.queued_stats();
            if stats.1 >= window {
                break;
            }
        }
    }
    let (chunks, capacity) = stats;
    assert!(chunks <= window / 16384 + 1, "{chunks} chunks");
    assert!(
        capacity <= window + 16384,
        "queued capacity {capacity} for a {window}-byte window"
    );
}

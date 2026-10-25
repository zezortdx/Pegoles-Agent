//! One host connection: handshake, then a mux stream per TCP connection.
//!
//! Threads (std, blocking): the session thread reads and decodes the vsock
//! channel; an accept thread takes Chromium's TCP connections; each stream
//! has a pump thread (TCP to mux, limited by send credit) and a drain thread
//! (mux to TCP, granting credit as the TCP writer drains). The session
//! thread never writes after the handshake, so two blocked writers cannot
//! deadlock each other. Memory per stream is bounded by the mux window.
//!
//! Lock order: `conn` and `streams` are never held together with `writer`;
//! an inbox lock may be taken inside `streams`, never the other way round.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pegoles_egress_proto::{Conn, Decoder, Event, Frame, Role, HEADER_LEN, MAX_PAYLOAD};

use crate::transport::Transport;
use crate::{der, policy_file, Config};

/// The host must send HELLO promptly or the slot is freed.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// A TCP client that stops reading for this long loses its stream.
const TCP_WRITE_TIMEOUT: Duration = Duration::from_secs(60);
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const THREAD_STACK: usize = 128 * 1024;

fn lock<X>(m: &Mutex<X>) -> MutexGuard<'_, X> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Bytes from the host waiting for the TCP writer. Bounded by the window:
/// the mux state machine rejects data beyond the credit it granted.
struct Inbox {
    state: Mutex<InboxState>,
    wake: Condvar,
}

struct InboxState {
    queue: VecDeque<Vec<u8>>,
    closed: bool,
    /// Drop queued data instead of flushing it (local close, shutdown).
    discard: bool,
}

impl Inbox {
    fn new() -> Inbox {
        Inbox {
            state: Mutex::new(InboxState {
                queue: VecDeque::new(),
                closed: false,
                discard: false,
            }),
            wake: Condvar::new(),
        }
    }

    fn push(&self, data: Vec<u8>) {
        lock(&self.state).queue.push_back(data);
        self.wake.notify_one();
    }

    fn close(&self, discard: bool) {
        let mut st = lock(&self.state);
        st.closed = true;
        st.discard |= discard;
        drop(st);
        self.wake.notify_all();
    }

    /// Next chunk to write; `None` once closed and (flushed or discarded).
    fn next(&self) -> Option<Vec<u8>> {
        let mut st = lock(&self.state);
        loop {
            if st.closed && st.discard {
                return None;
            }
            if let Some(chunk) = st.queue.pop_front() {
                return Some(chunk);
            }
            if st.closed {
                return None;
            }
            st = self.wake.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

struct Entry {
    inbox: Arc<Inbox>,
    tcp: TcpStream,
}

struct Shared<T: Transport> {
    conn: Mutex<Conn>,
    /// Signalled (with `conn`) when credit arrives or a stream/session ends.
    credit: Condvar,
    writer: Mutex<T>,
    /// Shutdown handle that never waits for the writer lock.
    ctl: Mutex<T>,
    streams: Mutex<HashMap<u32, Entry>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    down: AtomicBool,
}

impl<T: Transport> Shared<T> {
    fn is_down(&self) -> bool {
        self.down.load(Ordering::SeqCst)
    }

    /// Ends the session: wakes every blocked thread.
    fn kill(&self) {
        self.down.store(true, Ordering::SeqCst);
        lock(&self.ctl).shutdown();
        // Take the lock so a waiter cannot miss the flag between its check
        // and its wait.
        drop(lock(&self.conn));
        self.credit.notify_all();
    }

    fn send(&self, frame: &Frame) -> Result<(), ()> {
        if self.is_down() {
            return Err(());
        }
        let mut buf = Vec::with_capacity(HEADER_LEN + 16);
        frame.encode(&mut buf).map_err(|_| ())?;
        let mut w = lock(&self.writer);
        let res = w.write_all(&buf).and_then(|()| w.flush());
        drop(w);
        if res.is_err() {
            self.kill();
            return Err(());
        }
        Ok(())
    }

    /// Blocks until `id` has send credit; `None` if the stream or session
    /// is gone.
    fn wait_window(&self, id: u32) -> Option<usize> {
        let mut conn = lock(&self.conn);
        loop {
            if self.is_down() {
                return None;
            }
            match conn.send_window(id) {
                None => return None,
                Some(0) => {
                    conn = self
                        .credit
                        .wait(conn)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Some(n) => return Some(usize::try_from(n).unwrap_or(MAX_PAYLOAD)),
            }
        }
    }

    /// Closes a stream from our side (TCP EOF/error): CLOSE to the host,
    /// drop queued data, shut the TCP connection. No-op if the host already
    /// closed it (its queued data is still flushed by the drain thread).
    fn close_local(&self, id: u32) {
        let frame = lock(&self.conn).close_stream(id);
        self.credit.notify_all();
        let Some(frame) = frame else { return };
        if let Some(e) = lock(&self.streams).get(&id) {
            e.inbox.close(true);
            let _ = e.tcp.shutdown(Shutdown::Both);
        }
        let _ = self.send(&frame);
    }

    fn reap_workers(&self) {
        lock(&self.workers).retain(|h| !h.is_finished());
    }

    /// A new TCP connection from Chromium: open a stream for it. Dropping
    /// `tcp` (limit reached, any failure) closes it immediately.
    fn open_tcp(self: &Arc<Self>, tcp: TcpStream) {
        if tcp.set_nonblocking(false).is_err() {
            return;
        }
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_write_timeout(Some(TCP_WRITE_TIMEOUT));
        let (Ok(writer_half), Ok(ctl_half)) = (tcp.try_clone(), tcp.try_clone()) else {
            return;
        };
        self.reap_workers();
        let opened = lock(&self.conn).open_stream();
        let Ok((id, open_frame)) = opened else {
            log!("stream limit reached, connection refused");
            return;
        };
        let inbox = Arc::new(Inbox::new());
        // Registered before OPEN goes out: the host cannot answer earlier.
        lock(&self.streams).insert(
            id,
            Entry {
                inbox: inbox.clone(),
                tcp: ctl_half,
            },
        );
        if self.send(&open_frame).is_err() {
            return;
        }
        let pump = {
            let sh = self.clone();
            spawn("pump", move || sh.pump_out(id, tcp))
        };
        let drain = {
            let sh = self.clone();
            spawn("drain", move || sh.drain_in(id, inbox, writer_half))
        };
        match (pump, drain) {
            (Ok(p), Ok(d)) => lock(&self.workers).extend([p, d]),
            (p, d) => {
                log!("cannot start stream threads");
                self.close_local(id);
                lock(&self.workers).extend(p.into_iter().chain(d));
            }
        }
    }

    /// TCP to mux, never exceeding the credit the host granted.
    fn pump_out(&self, id: u32, mut tcp: TcpStream) {
        let mut buf = vec![0u8; MAX_PAYLOAD];
        while let Some(allowed) = self.wait_window(id) {
            let cap = allowed.min(MAX_PAYLOAD);
            let n = match tcp.read(&mut buf[..cap]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let reserved = lock(&self.conn).reserve_send(id, n);
            if reserved != Ok(n) {
                break;
            }
            let frame = Frame::Data {
                stream: id,
                payload: buf[..n].to_vec(),
            };
            if self.send(&frame).is_err() {
                break;
            }
        }
        self.close_local(id);
    }

    /// Mux to TCP; grants credit back as the TCP writer drains.
    fn drain_in(&self, id: u32, inbox: Arc<Inbox>, mut tcp: TcpStream) {
        while let Some(chunk) = inbox.next() {
            if tcp.write_all(&chunk).is_err() {
                self.close_local(id);
                break;
            }
            let granted = u32::try_from(chunk.len())
                .ok()
                .and_then(|n| lock(&self.conn).grant(id, n).ok());
            if let Some(frame) = granted {
                if self.send(&frame).is_err() {
                    break;
                }
            }
        }
        let _ = tcp.shutdown(Shutdown::Both);
        lock(&self.streams).remove(&id);
    }

    fn accept_loop(self: &Arc<Self>, listener: TcpListener) {
        if listener.set_nonblocking(true).is_err() {
            self.kill();
            return;
        }
        while !self.is_down() {
            match listener.accept() {
                Ok((tcp, _)) => self.open_tcp(tcp),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => thread::sleep(ACCEPT_POLL),
            }
        }
        // `listener` drops here: the port is closed.
    }

    /// Tears everything down: no TCP listener, connection or thread left.
    fn shutdown_all(&self, accept: Option<JoinHandle<()>>) {
        self.kill();
        if let Some(h) = accept {
            let _ = h.join();
        }
        for e in lock(&self.streams).values() {
            e.inbox.close(true);
            let _ = e.tcp.shutdown(Shutdown::Both);
        }
        let workers = std::mem::take(&mut *lock(&self.workers));
        for h in workers {
            let _ = h.join();
        }
    }
}

fn spawn<F: FnOnce() + Send + 'static>(name: &str, f: F) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name(name.into())
        .stack_size(THREAD_STACK)
        .spawn(f)
}

/// Resets the policy file when the session ends, panic included.
struct PolicyReset(PathBuf);

impl Drop for PolicyReset {
    fn drop(&mut self) {
        if let Err(e) = policy_file::reset(&self.0) {
            log!("cannot reset the policy file: {e}");
        }
    }
}

struct Session<'a, T: Transport> {
    sh: Arc<Shared<T>>,
    reader: T,
    cfg: &'a Config,
    accept: Option<JoinHandle<()>>,
}

impl<T: Transport> Session<'_, T> {
    /// Reads until EOF, an I/O error or a protocol violation.
    fn read_loop(&mut self) -> String {
        let mut decoder = Decoder::new();
        let mut buf = vec![0u8; MAX_PAYLOAD];
        loop {
            let n = match self.reader.read(&mut buf) {
                Ok(0) => return "host closed the channel".into(),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return format!("channel error: {e}"),
            };
            let mut input = &buf[..n];
            loop {
                match decoder.decode(&mut input) {
                    Ok(Some(frame)) => {
                        if let Err(why) = self.on_frame(frame) {
                            return why;
                        }
                    }
                    Ok(None) if input.is_empty() => break,
                    Ok(None) => {}
                    Err(e) => return format!("protocol violation: {e}"),
                }
            }
            if self.sh.is_down() {
                return "channel went down".into();
            }
        }
    }

    fn on_frame(&mut self, frame: Frame) -> Result<(), String> {
        let event = lock(&self.sh.conn)
            .on_frame(&frame)
            .map_err(|e| format!("protocol violation: {e}"))?;
        match (event, frame) {
            (Event::Hello, Frame::Hello { ca_der, .. }) => self.on_hello(&ca_der),
            (Event::Data(id), Frame::Data { payload, .. }) => {
                let inbox = lock(&self.sh.streams).get(&id).map(|e| e.inbox.clone());
                if let Some(inbox) = inbox {
                    inbox.push(payload);
                }
                Ok(())
            }
            (Event::Credit(_), _) => {
                self.sh.credit.notify_all();
                Ok(())
            }
            (Event::Closed(id), _) => {
                let inbox = lock(&self.sh.streams).get(&id).map(|e| e.inbox.clone());
                if let Some(inbox) = inbox {
                    // Flush what the host already sent, then close.
                    inbox.close(false);
                }
                self.sh.credit.notify_all();
                Ok(())
            }
            (Event::Ignored, _) => Ok(()),
            _ => Err("unexpected frame".into()),
        }
    }

    /// CA check and install, bind the TCP listener, HELLO_ACK, then accept.
    fn on_hello(&mut self, ca_der: &[u8]) -> Result<(), String> {
        der::validate_ca_certificate(ca_der)
            .map_err(|e| format!("rejected CA certificate: {e}"))?;
        policy_file::install(&self.cfg.policy_file, ca_der)
            .map_err(|e| format!("cannot write the policy file: {e}"))?;
        // Bind first: the host must never be told the channel is ready when
        // Chromium could not reach it.
        let listener = TcpListener::bind(self.cfg.proxy_addr)
            .map_err(|e| format!("cannot bind the proxy port: {e}"))?;
        let ack = lock(&self.sh.conn)
            .ack_frame()
            .map_err(|e| format!("protocol violation: {e}"))?;
        self.sh
            .send(&ack)
            .map_err(|()| "cannot send HELLO_ACK".to_string())?;
        if let (Some(tx), Ok(addr)) = (&self.cfg.listening, listener.local_addr()) {
            let _ = tx.send(addr);
        }
        let sh = self.sh.clone();
        self.accept = Some(
            spawn("accept", move || sh.accept_loop(listener))
                .map_err(|e| format!("cannot start the accept thread: {e}"))?,
        );
        let _ = self.reader.set_read_timeout(None);
        Ok(())
    }
}

/// Runs one host connection to its end. On return the TCP listener and all
/// TCP connections are closed and the policy file is reset.
pub fn run_session<T: Transport>(transport: T, cfg: &Config) {
    let _reset = PolicyReset(cfg.policy_file.clone());
    let (reader, ctl) = match (transport.try_clone(), transport.try_clone()) {
        (Ok(r), Ok(c)) => (r, c),
        _ => {
            log!("cannot duplicate the channel handle");
            return;
        }
    };
    if reader.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).is_err() {
        log!("cannot set the handshake timeout");
        return;
    }
    let sh = Arc::new(Shared {
        conn: Mutex::new(Conn::new(Role::Guest)),
        credit: Condvar::new(),
        writer: Mutex::new(transport),
        ctl: Mutex::new(ctl),
        streams: Mutex::new(HashMap::new()),
        workers: Mutex::new(Vec::new()),
        down: AtomicBool::new(false),
    });
    let mut session = Session {
        sh: sh.clone(),
        reader,
        cfg,
        accept: None,
    };
    let why = session.read_loop();
    log!("session ended: {why}");
    sh.shutdown_all(session.accept.take());
}

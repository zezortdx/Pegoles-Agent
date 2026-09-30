//! Mux session over any `AsyncRead + AsyncWrite` stream (protocol in
//! `pegoles-egress-proto`).
//!
//! One task owns the read side (decoder, protocol state, stream dispatch),
//! one owns the write side (a queue of encoded frames). Each guest stream is
//! a [`MuxStream`]: an `AsyncRead + AsyncWrite` whose reads drain the
//! stream's inbound queue (granting credit as data is consumed) and whose
//! writes wait for send credit. Queued bytes are bounded by the windows:
//! 64 streams x 256 KiB in each direction.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use pegoles_egress_proto::{Conn, Event, Frame, ProtoError, INITIAL_WINDOW, MAX_PAYLOAD};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

/// Bytes consumed before a CREDIT is sent back (a quarter window).
const CREDIT_BATCH: u32 = INITIAL_WINDOW / 4;

/// Capacity of every inbound chunk. Incoming DATA is copied into full
/// chunks, so queued memory tracks queued bytes (a peer sending 1-byte
/// frames cannot multiply it by the per-allocation overhead).
const CHUNK: usize = MAX_PAYLOAD;

/// Appends `data` to the queue, filling the tail chunk first.
fn enqueue(queue: &mut VecDeque<Vec<u8>>, mut data: &[u8]) {
    while !data.is_empty() {
        if !queue.back().is_some_and(|t| t.len() < CHUNK) {
            queue.push_back(Vec::with_capacity(CHUNK));
        }
        if let Some(tail) = queue.back_mut() {
            let n = (CHUNK - tail.len()).min(data.len());
            tail.extend_from_slice(&data[..n]);
            data = &data[n..];
        }
    }
}

/// What the read loop must do after a frame.
pub(crate) enum Step {
    None,
    /// A new stream to serve.
    Opened(MuxStream),
    /// The peer closed this stream: its handler must stop.
    Closed(u32),
}

#[derive(Default)]
struct Slot {
    queue: VecDeque<Vec<u8>>,
    /// Bytes already consumed from the front chunk.
    front_off: usize,
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
    /// Consumed but not yet granted back to the peer.
    consumed: u32,
    peer_closed: bool,
    local_closed: bool,
}

struct Inner {
    conn: Conn,
    slots: HashMap<u32, Slot>,
    dead: bool,
}

/// Shared between the session tasks and every stream.
pub(crate) struct Core {
    inner: Mutex<Inner>,
    out: mpsc::UnboundedSender<Vec<u8>>,
}

impl Core {
    pub(crate) fn new(conn: Conn, out: mpsc::UnboundedSender<Vec<u8>>) -> Arc<Core> {
        Arc::new(Core {
            inner: Mutex::new(Inner {
                conn,
                slots: HashMap::new(),
                dead: false,
            }),
            out,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a stream task panicked; the state is
        // still consistent enough to tear everything down.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn send(&self, frame: &Frame) -> bool {
        match frame.to_bytes() {
            Ok(b) => self.out.send(b).is_ok(),
            Err(_) => false,
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.lock().conn.is_ready()
    }

    /// Host handshake step: the HELLO frame to send first.
    pub(crate) fn hello_frame(&self, ca_der: Vec<u8>) -> Result<Frame, ProtoError> {
        self.lock().conn.hello_frame(ca_der)
    }

    /// Applies an incoming frame.
    pub(crate) fn on_frame(self: &Arc<Self>, frame: Frame) -> Result<Step, ProtoError> {
        let mut g = self.lock();
        let event = g.conn.on_frame(&frame)?;
        match event {
            Event::Opened(id) => {
                g.slots.insert(id, Slot::default());
                return Ok(Step::Opened(MuxStream {
                    id,
                    core: self.clone(),
                }));
            }
            Event::Data(id) => {
                if let (Frame::Data { payload, .. }, Some(slot)) = (frame, g.slots.get_mut(&id)) {
                    enqueue(&mut slot.queue, &payload);
                    if let Some(w) = slot.read_waker.take() {
                        w.wake();
                    }
                }
            }
            Event::Credit(id) => {
                if let Some(w) = g.slots.get_mut(&id).and_then(|s| s.write_waker.take()) {
                    w.wake();
                }
            }
            Event::Closed(id) => {
                if let Some(slot) = g.slots.get_mut(&id) {
                    slot.peer_closed = true;
                    if let Some(w) = slot.read_waker.take() {
                        w.wake();
                    }
                    if let Some(w) = slot.write_waker.take() {
                        w.wake();
                    }
                }
                return Ok(Step::Closed(id));
            }
            Event::Hello | Event::HelloAck | Event::Ignored => {}
        }
        Ok(Step::None)
    }

    /// Test guest: opens a stream.
    #[cfg(test)]
    pub(crate) fn open_stream(self: &Arc<Self>) -> Result<MuxStream, ProtoError> {
        let mut g = self.lock();
        let (id, frame) = g.conn.open_stream()?;
        g.slots.insert(id, Slot::default());
        self.send(&frame);
        Ok(MuxStream {
            id,
            core: self.clone(),
        })
    }

    /// Test guest: HELLO_ACK after an accepted HELLO.
    #[cfg(test)]
    pub(crate) fn guest_ack(&self) -> Result<Frame, ProtoError> {
        self.lock().conn.ack_frame()
    }

    /// Test guest: raw bytes on the wire (protocol-violation tests).
    #[cfg(test)]
    pub(crate) fn send_raw(&self, bytes: Vec<u8>) {
        let _ = self.out.send(bytes);
    }

    /// The session is over: wake everything, fail further writes.
    pub(crate) fn kill(&self) {
        let mut g = self.lock();
        g.dead = true;
        for slot in g.slots.values_mut() {
            slot.peer_closed = true;
            if let Some(w) = slot.read_waker.take() {
                w.wake();
            }
            if let Some(w) = slot.write_waker.take() {
                w.wake();
            }
        }
    }
}

/// One guest stream.
pub struct MuxStream {
    id: u32,
    core: Arc<Core>,
}

impl AsyncRead for MuxStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let core = &this.core;
        let mut g = core.lock();
        let Inner { conn, slots, dead } = &mut *g;
        let Some(slot) = slots.get_mut(&this.id) else {
            return Poll::Ready(Ok(()));
        };
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if let Some(front) = slot.queue.front() {
            let avail = &front[slot.front_off..];
            let n = avail.len().min(buf.remaining());
            buf.put_slice(&avail[..n]);
            slot.front_off += n;
            if slot.front_off == front.len() {
                slot.queue.pop_front();
                slot.front_off = 0;
            }
            // n <= 16 KiB, so the cast is lossless.
            slot.consumed = slot.consumed.saturating_add(n as u32);
            if slot.consumed >= CREDIT_BATCH && !slot.peer_closed && !slot.local_closed {
                let grant = std::mem::take(&mut slot.consumed);
                if let Ok(frame) = conn.grant(this.id, grant) {
                    core.send(&frame);
                }
            }
            return Poll::Ready(Ok(()));
        }
        if slot.peer_closed || slot.local_closed || *dead {
            return Poll::Ready(Ok(()));
        }
        slot.read_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for MuxStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let core = &this.core;
        let mut g = core.lock();
        let Inner { conn, slots, dead } = &mut *g;
        let broken = || io::Error::new(io::ErrorKind::BrokenPipe, "egress stream closed");
        let Some(slot) = slots.get_mut(&this.id) else {
            return Poll::Ready(Err(broken()));
        };
        if *dead || slot.peer_closed || slot.local_closed || !conn.is_open(this.id) {
            return Poll::Ready(Err(broken()));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = match conn.reserve_send(this.id, buf.len()) {
            Ok(n) => n,
            Err(_) => return Poll::Ready(Err(broken())),
        };
        if n == 0 {
            slot.write_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let frame = Frame::Data {
            stream: this.id,
            payload: buf[..n].to_vec(),
        };
        if !core.send(&frame) {
            return Poll::Ready(Err(broken()));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    /// Closes the stream in both directions (half-close is not supported).
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.close();
        Poll::Ready(Ok(()))
    }
}

impl MuxStream {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// Test probe: (queued chunks, total queued capacity in bytes).
    #[cfg(test)]
    pub(crate) fn queued_stats(&self) -> (usize, usize) {
        let g = self.core.lock();
        g.slots.get(&self.id).map_or((0, 0), |s| {
            (s.queue.len(), s.queue.iter().map(Vec::capacity).sum())
        })
    }

    fn close(&self) {
        let mut g = self.core.lock();
        if let Some(frame) = g.conn.close_stream(self.id) {
            self.core.send(&frame);
        }
        if let Some(slot) = g.slots.get_mut(&self.id) {
            slot.local_closed = true;
        }
    }
}

impl Drop for MuxStream {
    fn drop(&mut self) {
        let mut g = self.core.lock();
        if let Some(frame) = g.conn.close_stream(self.id) {
            self.core.send(&frame);
        }
        g.slots.remove(&self.id);
    }
}

mod session;

pub use session::SessionEnd;
pub(crate) use session::{start_host, Handler, HandshakeError, Running};

#[cfg(test)]
mod tests;

//! Host side of the mux connection: handshake, read loop, write loop.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use pegoles_egress_proto::{Conn, Decoder, ProtoError, Role};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};

use pegoles_egress_proto::MAX_STREAMS;

use super::{Core, MuxStream, Step};

/// HELLO_ACK must arrive within this.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs one accepted stream to completion.
pub(crate) type Handler =
    Arc<dyn Fn(MuxStream) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// `shutdown()` or the handle was dropped.
    Shutdown,
    /// The guest side closed the channel.
    PeerClosed,
    /// The guest violated the protocol; the connection was dropped.
    Protocol(ProtoError),
    Io(io::ErrorKind),
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("the guest did not answer HELLO in time")]
    Timeout,
    #[error("handshake I/O failed: {0}")]
    Io(io::ErrorKind),
    #[error("the guest violated the protocol during the handshake: {0}")]
    Protocol(ProtoError),
    #[error("the channel closed during the handshake")]
    Closed,
}

/// A running session. Dropping it aborts every task, which closes every
/// stream and so every upstream connection.
pub(crate) struct Running {
    task: Option<JoinHandle<()>>,
    shutdown: Option<oneshot::Sender<()>>,
    ended: watch::Receiver<Option<SessionEnd>>,
}

impl Running {
    pub(crate) fn is_closed(&self) -> bool {
        self.ended.borrow().is_some()
    }

    pub(crate) async fn closed(&self) -> SessionEnd {
        let mut rx = self.ended.clone();
        loop {
            if let Some(end) = *rx.borrow_and_update() {
                return end;
            }
            if rx.changed().await.is_err() {
                return (*rx.borrow()).unwrap_or(SessionEnd::Shutdown);
            }
        }
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Publishes the end of the session and kills the streams, also when the
/// task is aborted.
struct EndGuard {
    core: Arc<Core>,
    tx: watch::Sender<Option<SessionEnd>>,
    end: Option<SessionEnd>,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.core.kill();
        let _ = self.tx.send(Some(self.end.unwrap_or(SessionEnd::Shutdown)));
    }
}

/// Sends HELLO with the CA, waits for HELLO_ACK, then serves streams with
/// `handler` until the channel closes.
pub(crate) async fn start_host<S>(
    mut io: S,
    ca_der: Vec<u8>,
    handler: Handler,
) -> Result<Running, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let core = Core::new(Conn::new(Role::Host), out_tx);
    let (decoder, leftover) =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut io, &core, ca_der)).await {
            Ok(r) => r?,
            Err(_) => return Err(HandshakeError::Timeout),
        };

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ended_tx, ended_rx) = watch::channel(None);
    let task = tokio::spawn(async move {
        let mut guard = EndGuard {
            core: core.clone(),
            tx: ended_tx,
            end: None,
        };
        let (rd, wr) = tokio::io::split(io);
        let end = tokio::select! {
            e = read_loop(rd, decoder, leftover, core, handler) => e,
            e = write_loop(wr, out_rx) => e,
            _ = shutdown_rx => SessionEnd::Shutdown,
        };
        guard.end = Some(end);
    });
    Ok(Running {
        task: Some(task),
        shutdown: Some(shutdown_tx),
        ended: ended_rx,
    })
}

async fn handshake<S>(
    io: &mut S,
    core: &Arc<Core>,
    ca_der: Vec<u8>,
) -> Result<(Decoder, Vec<u8>), HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello = core
        .hello_frame(ca_der)
        .map_err(HandshakeError::Protocol)?
        .to_bytes()
        .map_err(HandshakeError::Protocol)?;
    io.write_all(&hello)
        .await
        .map_err(|e| HandshakeError::Io(e.kind()))?;
    io.flush().await.map_err(|e| HandshakeError::Io(e.kind()))?;

    let mut decoder = Decoder::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let n = io
            .read(&mut buf)
            .await
            .map_err(|e| HandshakeError::Io(e.kind()))?;
        if n == 0 {
            return Err(HandshakeError::Closed);
        }
        let mut input = &buf[..n];
        while let Some(frame) = decoder
            .decode(&mut input)
            .map_err(HandshakeError::Protocol)?
        {
            core.on_frame(frame).map_err(HandshakeError::Protocol)?;
            if core.is_ready() {
                return Ok((decoder, input.to_vec()));
            }
        }
    }
}

async fn read_loop<R>(
    mut rd: R,
    mut decoder: Decoder,
    leftover: Vec<u8>,
    core: Arc<Core>,
    handler: Handler,
) -> SessionEnd
where
    R: AsyncRead + Unpin,
{
    let mut handlers = Handlers::new(handler);
    if let Err(e) = feed(&mut decoder, &leftover, &core, &mut handlers) {
        return SessionEnd::Protocol(e);
    }
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        tokio::select! {
            r = rd.read(&mut buf) => match r {
                Ok(0) => return SessionEnd::PeerClosed,
                Ok(n) => {
                    if let Err(e) = feed(&mut decoder, &buf[..n], &core, &mut handlers) {
                        return SessionEnd::Protocol(e);
                    }
                }
                Err(e) => return SessionEnd::Io(e.kind()),
            },
            Some(_) = handlers.tasks.join_next(), if !handlers.tasks.is_empty() => {}
        }
    }
}

/// The running stream handlers. A handler holds one of `MAX_STREAMS`
/// permits until its task is gone, so a guest that opens and closes
/// streams in a loop cannot pile up tasks, sockets or lookups behind the
/// mux's own stream count (which a CLOSE frees at once). A peer CLOSE
/// aborts the handler, which drops its upstream connections.
struct Handlers {
    handler: Handler,
    tasks: JoinSet<()>,
    permits: Arc<Semaphore>,
    live: HashMap<u32, AbortHandle>,
}

impl Handlers {
    fn new(handler: Handler) -> Handlers {
        Handlers {
            handler,
            tasks: JoinSet::new(),
            permits: Arc::new(Semaphore::new(MAX_STREAMS)),
            live: HashMap::new(),
        }
    }

    /// Serves `stream`, or refuses it (dropping it sends CLOSE) while every
    /// permit is still held by a handler that has not finished.
    fn spawn(&mut self, stream: MuxStream) {
        self.live.retain(|_, h| !h.is_finished());
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            return;
        };
        let id = stream.id();
        let fut = (self.handler)(stream);
        let abort = self.tasks.spawn(async move {
            let _permit = permit;
            fut.await;
        });
        self.live.insert(id, abort);
    }

    fn peer_closed(&mut self, id: u32) {
        if let Some(h) = self.live.remove(&id) {
            h.abort();
        }
    }
}

fn feed(
    decoder: &mut Decoder,
    data: &[u8],
    core: &Arc<Core>,
    handlers: &mut Handlers,
) -> Result<(), ProtoError> {
    let mut input = data;
    loop {
        match decoder.decode(&mut input)? {
            Some(frame) => match core.on_frame(frame)? {
                Step::Opened(stream) => handlers.spawn(stream),
                Step::Closed(id) => handlers.peer_closed(id),
                Step::None => {}
            },
            None if input.is_empty() => return Ok(()),
            None => {}
        }
    }
}

async fn write_loop<W>(mut wr: W, mut rx: mpsc::UnboundedReceiver<Vec<u8>>) -> SessionEnd
where
    W: AsyncWrite + Unpin,
{
    while let Some(first) = rx.recv().await {
        if let Err(e) = wr.write_all(&first).await {
            return SessionEnd::Io(e.kind());
        }
        while let Ok(more) = rx.try_recv() {
            if let Err(e) = wr.write_all(&more).await {
                return SessionEnd::Io(e.kind());
            }
        }
        if let Err(e) = wr.flush().await {
            return SessionEnd::Io(e.kind());
        }
    }
    SessionEnd::Shutdown
}

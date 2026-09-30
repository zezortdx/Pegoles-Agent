//! Request/response relay over one guest connection and one upstream
//! connection: policy per request, response inspection per response,
//! keep-alive, chunked re-framing, WebSocket pass-through after a 101.

use std::future::Future;
use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use super::body::{BodyReader, Encoding};
use super::http::{
    has_token, host_header, is_websocket_request, request_framing, response_framing, single,
    wants_close, Buffered, Framing, HttpError, RequestHead, ResponseHead,
};
use super::upstream::{self, BoxedIo};
use super::{refuse, Ctx, IDLE};
use crate::inspect::{self, blocked_response, ResponseMeta, Verdict};
use crate::policy::{path_only, Decision, DenyReason, Scheme};

/// Request body cap.
pub const MAX_REQUEST_BODY: u64 = 10 * 1024 * 1024;
/// Interim (1xx) responses tolerated before the real one.
const MAX_INTERIM: usize = 8;

/// Where requests go.
pub(super) struct Route {
    pub host: String,
    pub scheme: Scheme,
    pub port: u16,
    pub addrs: Vec<SocketAddr>,
}

impl Route {
    fn default_port(&self) -> u16 {
        match self.scheme {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flow {
    Continue,
    Close,
}

pub(super) type Upstream = Buffered<BoxedIo>;

/// Write with the idle timeout.
async fn timed<F: Future<Output = io::Result<()>>>(f: F) -> io::Result<()> {
    match tokio::time::timeout(IDLE, f).await {
        Ok(r) => r,
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "write stalled")),
    }
}

enum SendError {
    /// Upstream failed before any response byte; safe to retry a bodiless
    /// request on a fresh connection.
    Retryable,
    Upstream,
    Deny(DenyReason),
    Guest,
}

struct Outcome {
    flow: Flow,
    reuse: bool,
    decision: Decision,
}

impl Outcome {
    fn closed(decision: Decision) -> Outcome {
        Outcome {
            flow: Flow::Close,
            reuse: false,
            decision,
        }
    }
}

/// One request/response exchange. `origin` is the upstream request target,
/// `path` the query-free path used for the URL rules and the audit.
pub(super) async fn exchange<G>(
    ctx: &Ctx,
    guest: &mut Buffered<G>,
    up: &mut Option<Upstream>,
    route: &Route,
    req: RequestHead,
    origin: &str,
) -> Flow
where
    G: AsyncRead + AsyncWrite + Unpin,
{
    let path = path_only(origin);
    if let Decision::Deny(r) = ctx.policy.evaluate_url(&route.host, &path) {
        refuse(ctx, &mut guest.io, &route.host, &path, r).await;
        return Flow::Close;
    }
    let framing = match request_framing(&req, MAX_REQUEST_BODY) {
        Ok(f) => f,
        Err(r) => {
            refuse(ctx, &mut guest.io, &route.host, &path, r).await;
            return Flow::Close;
        }
    };
    let ws = is_websocket_request(&req);
    let head_bytes = super::http::serialize_request(
        &req,
        origin,
        &host_header(&route.host, route.port, route.default_port()),
        framing,
        ws,
    );
    let guest_close = wants_close(req.minor, &req.headers);
    let mut bytes = 0u64;

    let mut attempt = 0;
    let resp = loop {
        let fresh = up.is_none();
        if fresh {
            match upstream::connect(ctx, route.scheme, &route.host, &route.addrs).await {
                Ok(u) => *up = Some(u),
                Err(r) => {
                    refuse(ctx, &mut guest.io, &route.host, &path, r).await;
                    return Flow::Close;
                }
            }
        }
        let Some(u) = up.as_mut() else {
            return Flow::Close;
        };
        match send_request(ctx, guest, u, &head_bytes, framing, &mut bytes).await {
            Ok(resp) => break resp,
            Err(SendError::Retryable) if !fresh && attempt == 0 && framing == Framing::None => {
                *up = None;
                attempt += 1;
            }
            Err(SendError::Retryable | SendError::Upstream) => {
                *up = None;
                refuse(
                    ctx,
                    &mut guest.io,
                    &route.host,
                    &path,
                    DenyReason::UpstreamProtocol,
                )
                .await;
                return Flow::Close;
            }
            Err(SendError::Deny(r)) => {
                *up = None;
                refuse(ctx, &mut guest.io, &route.host, &path, r).await;
                return Flow::Close;
            }
            Err(SendError::Guest) => {
                *up = None;
                return Flow::Close;
            }
        }
    };

    let Some(u) = up.as_mut() else {
        return Flow::Close;
    };
    let out = relay_response(
        ctx,
        guest,
        u,
        req.method == "HEAD",
        ws,
        resp,
        guest_close,
        &mut bytes,
    )
    .await;
    // Bytes left in the upstream buffer are a desync (pipelined or injected
    // data): never reuse that connection.
    if !out.reuse || !u.unread().is_empty() {
        *up = None;
    }
    ctx.emit(&route.host, &path, out.decision, bytes);
    out.flow
}

/// Sends the request (head and body) and reads the final response head.
async fn send_request<G>(
    ctx: &Ctx,
    guest: &mut Buffered<G>,
    u: &mut Upstream,
    head: &[u8],
    framing: Framing,
    bytes: &mut u64,
) -> Result<ResponseHead, SendError>
where
    G: AsyncRead + AsyncWrite + Unpin,
{
    let no_body = framing == Framing::None;
    let failed_write = || {
        if no_body {
            SendError::Retryable
        } else {
            SendError::Upstream
        }
    };
    timed(u.io.write_all(head))
        .await
        .map_err(|_| failed_write())?;
    if !no_body {
        let enc = Encoding::for_framing(framing);
        let mut reader = BodyReader::new(framing);
        let mut total = 0u64;
        loop {
            let piece = match reader.next(guest, IDLE).await {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(_) => return Err(SendError::Guest),
            };
            total += piece.len() as u64;
            if total > MAX_REQUEST_BODY {
                return Err(SendError::Deny(DenyReason::BodyTooLarge));
            }
            if !ctx.budget.charge(piece.len() as u64) {
                return Err(SendError::Deny(DenyReason::SessionByteCap));
            }
            timed(enc.write(&mut u.io, &piece))
                .await
                .map_err(|_| SendError::Upstream)?;
        }
        timed(enc.finish(&mut u.io))
            .await
            .map_err(|_| SendError::Upstream)?;
        *bytes += total;
    }
    timed(u.io.flush()).await.map_err(|_| failed_write())?;

    for _ in 0..=MAX_INTERIM {
        match u.read_response_head(IDLE).await {
            Ok(h) if (100..200).contains(&h.status) && h.status != 101 => continue,
            Ok(h) => return Ok(h),
            Err(HttpError::Eof | HttpError::Io) if u.unread().is_empty() && no_body => {
                return Err(SendError::Retryable)
            }
            Err(_) => return Err(SendError::Upstream),
        }
    }
    Err(SendError::Upstream)
}

#[allow(clippy::too_many_arguments)]
async fn relay_response<G>(
    ctx: &Ctx,
    guest: &mut Buffered<G>,
    u: &mut Upstream,
    head_request: bool,
    ws: bool,
    resp: ResponseHead,
    guest_close: bool,
    bytes: &mut u64,
) -> Outcome
where
    G: AsyncRead + AsyncWrite + Unpin,
{
    let allow = Decision::Allow;
    let upstream_bad = Decision::Deny(DenyReason::UpstreamProtocol);

    if resp.status == 101 {
        let is_ws = ws
            && has_token(&resp.headers, "connection", "upgrade")
            && matches!(single(&resp.headers, "upgrade"), Ok(Some(v))
                if String::from_utf8_lossy(v).trim().eq_ignore_ascii_case("websocket"));
        if !is_ws {
            let _ = timed(
                guest
                    .io
                    .write_all(&inspect::refusal_response(DenyReason::UpstreamProtocol)),
            )
            .await;
            return Outcome::closed(upstream_bad);
        }
        let head = super::http::serialize_response(&resp, Framing::None, false, true);
        if timed(guest.io.write_all(&head)).await.is_err() {
            return Outcome::closed(allow);
        }
        pipe_websocket(ctx, guest, u, bytes).await;
        return Outcome::closed(allow);
    }

    let framing = match response_framing(head_request, &resp) {
        Ok(f) => f,
        Err(_) => {
            let _ = timed(
                guest
                    .io
                    .write_all(&inspect::refusal_response(DenyReason::UpstreamProtocol)),
            )
            .await;
            return Outcome::closed(upstream_bad);
        }
    };

    // A repeated type/disposition/encoding header is ambiguous: a client may
    // honour a different copy than the one inspected, so it fails closed.
    let (Ok(ctype), Ok(cdisp), Ok(cenc)) = (
        single(&resp.headers, "content-type"),
        single(&resp.headers, "content-disposition"),
        single(&resp.headers, "content-encoding"),
    ) else {
        let _ = timed(
            guest
                .io
                .write_all(&inspect::refusal_response(DenyReason::UpstreamProtocol)),
        )
        .await;
        return Outcome::closed(upstream_bad);
    };
    let meta = ResponseMeta::from_headers(
        ctype,
        cdisp,
        match framing {
            Framing::Length(n) => Some(n),
            _ => None,
        },
        cenc,
    );
    let has_body = framing != Framing::None;
    if has_body {
        if let Verdict::Block(r) = inspect::precheck(&meta) {
            return block(guest, r).await;
        }
    }

    // Buffer the head of the body, inspect, then release.
    let mut reader = BodyReader::new(framing);
    let mut held: Vec<u8> = Vec::new();
    if has_body {
        let need = inspect::head_needed(&meta);
        while held.len() < need && !reader.is_done() {
            match reader.next(u, IDLE).await {
                Ok(Some(p)) => held.extend_from_slice(&p),
                Ok(None) => break,
                Err(_) => {
                    let _ = timed(
                        guest
                            .io
                            .write_all(&inspect::refusal_response(DenyReason::UpstreamProtocol)),
                    )
                    .await;
                    return Outcome::closed(upstream_bad);
                }
            }
        }
        let complete = reader.is_done();
        if let Verdict::Block(r) = inspect::inspect_head(&meta, &held, complete) {
            return block(guest, r).await;
        }
    }

    let upstream_close = wants_close(resp.minor, &resp.headers) || framing == Framing::UntilClose;
    let close = guest_close || upstream_close;
    let head = super::http::serialize_response(&resp, framing, guest_close, false);
    if timed(guest.io.write_all(&head)).await.is_err() {
        return Outcome::closed(allow);
    }
    let enc = Encoding::for_framing(framing);
    let cap = inspect::stream_cap(&meta);
    let mut total = 0u64;
    let mut pending = Some(held);
    loop {
        let piece = match pending.take() {
            Some(p) => p,
            None => match reader.next(u, IDLE).await {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(_) => return Outcome::closed(allow),
            },
        };
        if piece.is_empty() {
            continue;
        }
        total += piece.len() as u64;
        if cap.is_some_and(|c| total > c) {
            *bytes += total;
            return Outcome::closed(Decision::Deny(DenyReason::DownloadTooLarge));
        }
        if !ctx.budget.charge(piece.len() as u64) {
            *bytes += total;
            return Outcome::closed(Decision::Deny(DenyReason::SessionByteCap));
        }
        if timed(enc.write(&mut guest.io, &piece)).await.is_err() {
            *bytes += total;
            return Outcome::closed(allow);
        }
        if reader.is_done() {
            break;
        }
    }
    *bytes += total;
    if timed(enc.finish(&mut guest.io)).await.is_err() || timed(guest.io.flush()).await.is_err() {
        return Outcome::closed(allow);
    }
    Outcome {
        flow: if close { Flow::Close } else { Flow::Continue },
        reuse: !upstream_close && reader.is_done(),
        decision: allow,
    }
}

async fn block<G>(guest: &mut Buffered<G>, reason: DenyReason) -> Outcome
where
    G: AsyncRead + AsyncWrite + Unpin,
{
    let _ = timed(guest.io.write_all(&blocked_response())).await;
    let _ = timed(guest.io.flush()).await;
    Outcome::closed(Decision::Deny(reason))
}

/// Raw two-way copy after a 101, until either side closes or stalls.
async fn pipe_websocket<G>(ctx: &Ctx, guest: &mut Buffered<G>, u: &mut Upstream, bytes: &mut u64)
where
    G: AsyncRead + AsyncWrite + Unpin,
{
    enum Ev {
        Guest(Result<usize, HttpError>),
        Up(Result<usize, HttpError>),
    }
    // Bytes that arrived with the handshake.
    if !u.unread().is_empty() {
        let n = u.unread().len();
        *bytes += n as u64;
        if !ctx.budget.charge(n as u64) || timed(guest.io.write_all(u.unread())).await.is_err() {
            return;
        }
        u.consume(n);
    }
    loop {
        if !guest.unread().is_empty() {
            let n = guest.unread().len();
            *bytes += n as u64;
            if !ctx.budget.charge(n as u64)
                || timed(u.io.write_all(guest.unread())).await.is_err()
                || timed(u.io.flush()).await.is_err()
            {
                return;
            }
            guest.consume(n);
        }
        let ev = tokio::select! {
            r = guest.fill(IDLE) => Ev::Guest(r),
            r = u.fill(IDLE) => Ev::Up(r),
        };
        match ev {
            Ev::Guest(Ok(n)) if n > 0 => {}
            Ev::Up(Ok(n)) if n > 0 => {
                let n = u.unread().len();
                *bytes += n as u64;
                if !ctx.budget.charge(n as u64)
                    || timed(guest.io.write_all(u.unread())).await.is_err()
                    || timed(guest.io.flush()).await.is_err()
                {
                    return;
                }
                u.consume(n);
            }
            _ => return,
        }
    }
}

/// Tunnel mode: requests arrive as origin-form over the intercepted TLS
/// stream; the `Host` header must be the CONNECT host.
pub(super) async fn tunnel_loop<G>(
    ctx: &Ctx,
    mut guest: Buffered<G>,
    mut up: Option<Upstream>,
    route: Route,
) where
    G: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let req = match guest.read_request_head(IDLE).await {
            Ok(Some(r)) => r,
            Ok(None) | Err(HttpError::Io | HttpError::Eof | HttpError::Timeout) => break,
            Err(HttpError::HeadTooLarge) => {
                refuse(
                    ctx,
                    &mut guest.io,
                    &route.host,
                    "",
                    DenyReason::HeadTooLarge,
                )
                .await;
                break;
            }
            Err(HttpError::Malformed) => {
                refuse(
                    ctx,
                    &mut guest.io,
                    &route.host,
                    "",
                    DenyReason::MalformedRequest,
                )
                .await;
                break;
            }
        };
        if !req.target.starts_with('/') {
            refuse(
                ctx,
                &mut guest.io,
                &route.host,
                "",
                DenyReason::MalformedRequest,
            )
            .await;
            break;
        }
        if !host_matches(&req, &route) {
            refuse(
                ctx,
                &mut guest.io,
                &route.host,
                &path_only(&req.target),
                DenyReason::HostMismatch,
            )
            .await;
            break;
        }
        let origin = req.target.clone();
        if exchange(ctx, &mut guest, &mut up, &route, req, &origin).await == Flow::Close {
            break;
        }
    }
    let _ = guest.io.shutdown().await;
}

/// `Host` names the CONNECT host (port 443 optional).
fn host_matches(req: &RequestHead, route: &Route) -> bool {
    let Ok(Some(v)) = single(&req.headers, "host") else {
        return false;
    };
    let v = String::from_utf8_lossy(v).to_ascii_lowercase();
    let v = v.trim();
    v == route.host || v.strip_suffix(":443") == Some(route.host.as_str())
}

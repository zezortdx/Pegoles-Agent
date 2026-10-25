//! The proxy served on every guest stream: parse the proxy request, run the
//! policy, resolve and connect to the checked address, intercept TLS for
//! CONNECT, relay with inspection, audit.

mod body;
mod http;
mod relay;
mod upstream;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;

use crate::audit::{AuditEvent, AuditSink};
use crate::inspect::refusal_response;
use crate::mux::MuxStream;
use crate::policy::{
    parse_proxy_request, resolve_checked, Decision, DenyReason, Policy, Resolver, Scheme,
};
use crate::tls::SessionCa;
use http::{Buffered, HttpError};
use relay::{Flow, Route};

#[cfg(test)]
pub(crate) use upstream::{BoxedIo, ConnectFuture};
pub(crate) use upstream::{Connector, TcpConnector};

/// Idle timeout for any read or write that makes no progress.
pub const IDLE: Duration = Duration::from_secs(60);
/// Bytes relayed per session (request and response bodies together).
pub const SESSION_BYTE_CAP: u64 = 1024 * 1024 * 1024;

/// Bytes budget shared by every stream of a session.
pub(crate) struct Budget {
    used: AtomicU64,
    cap: u64,
}

impl Budget {
    pub fn new(cap: u64) -> Budget {
        Budget {
            used: AtomicU64::new(0),
            cap,
        }
    }

    /// Accounts `n` bytes; false once the cap is exceeded.
    pub fn charge(&self, n: u64) -> bool {
        let prev = self.used.fetch_add(n, Ordering::Relaxed);
        prev.saturating_add(n) <= self.cap
    }
}

/// Everything a stream handler needs, fixed for the session.
pub(crate) struct Ctx {
    pub policy: Policy,
    pub resolver: Arc<dyn Resolver>,
    pub connector: Arc<dyn Connector>,
    pub ca: SessionCa,
    pub upstream_tls: Arc<ClientConfig>,
    pub audit: AuditSink,
    pub budget: Budget,
}

impl Ctx {
    pub fn emit(&self, host: &str, path: &str, decision: Decision, bytes: u64) {
        (self.audit)(AuditEvent::new(
            self.policy.mode_kind(),
            host,
            path,
            decision,
            bytes,
        ));
    }
}

/// Audits a refusal and answers with the matching static page.
pub(crate) async fn refuse<W: AsyncWrite + Unpin>(
    ctx: &Ctx,
    w: &mut W,
    host: &str,
    path: &str,
    reason: DenyReason,
) {
    ctx.emit(host, path, Decision::Deny(reason), 0);
    let write = async {
        w.write_all(&refusal_response(reason)).await?;
        w.flush().await?;
        w.shutdown().await
    };
    let _ = tokio::time::timeout(IDLE, write).await;
}

/// Serves one guest stream to completion.
pub(crate) async fn serve(ctx: Arc<Ctx>, stream: MuxStream) {
    let mut guest = Buffered::new(stream);
    let head = match guest.read_request_head(IDLE).await {
        Ok(Some(h)) => h,
        Ok(None) | Err(HttpError::Io | HttpError::Eof | HttpError::Timeout) => return,
        Err(HttpError::HeadTooLarge) => {
            refuse(&ctx, &mut guest.io, "", "", DenyReason::HeadTooLarge).await;
            return;
        }
        Err(HttpError::Malformed) => {
            refuse(&ctx, &mut guest.io, "", "", DenyReason::MalformedRequest).await;
            return;
        }
    };
    if head.method == "CONNECT" {
        tunnel(&ctx, guest, head).await;
    } else {
        plain(&ctx, guest, head).await;
    }
}

/// Raw host text for the audit of a request that failed validation. Never
/// carries userinfo: everything up to the last `@` of the authority is
/// dropped, a `Userinfo` denial audits no host at all, and so does a target
/// with an `@` past the authority (a password containing `/`, whose first
/// half would otherwise read as the authority).
fn raw_host(target: &str, reason: DenyReason) -> &str {
    let t = target.split_once("://").map_or(target, |(_, r)| r);
    let authority = t.split(['/', '?', '#']).next().unwrap_or("");
    if reason == DenyReason::Userinfo || (t.contains('@') && !authority.contains('@')) {
        return "";
    }
    authority.rsplit_once('@').map_or(authority, |(_, h)| h)
}

/// `CONNECT host:443`: policy, resolve, upstream TLS, then intercept.
async fn tunnel(ctx: &Ctx, mut guest: Buffered<MuxStream>, head: http::RequestHead) {
    let target = match parse_proxy_request(&head.method, &head.target) {
        Ok(t) => t,
        Err(r) => {
            refuse(ctx, &mut guest.io, raw_host(&head.target, r), "", r).await;
            return;
        }
    };
    let host = target.host.clone();
    if let Decision::Deny(r) = ctx.policy.evaluate_host(&host) {
        refuse(ctx, &mut guest.io, &host, "", r).await;
        return;
    }
    // The client waits for our 200 before it speaks TLS.
    if !guest.unread().is_empty() {
        refuse(ctx, &mut guest.io, &host, "", DenyReason::MalformedRequest).await;
        return;
    }
    let addrs = match resolve_checked(&*ctx.resolver, &host, target.port).await {
        Ok(a) => a,
        Err(r) => {
            refuse(ctx, &mut guest.io, &host, "", r).await;
            return;
        }
    };
    let up = match upstream::connect(ctx, Scheme::Https, &host, &addrs).await {
        Ok(u) => u,
        Err(r) => {
            refuse(ctx, &mut guest.io, &host, "", r).await;
            return;
        }
    };
    let server_config = match ctx.ca.server_config(&host) {
        Ok(c) => c,
        Err(_) => {
            refuse(ctx, &mut guest.io, &host, "", DenyReason::UpstreamTls).await;
            return;
        }
    };
    let ok = guest
        .io
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    if tokio::time::timeout(IDLE, ok).await.is_err() {
        return;
    }
    ctx.emit(&host, "", Decision::Allow, 0);

    let accept = TlsAcceptor::from(server_config).accept(guest.io);
    let tls = match tokio::time::timeout(upstream::HANDSHAKE_TIMEOUT, accept).await {
        Ok(Ok(s)) => s,
        _ => return,
    };
    // Domain-fronting check: SNI, if sent, must be the CONNECT host.
    if let Some(sni) = tls.get_ref().1.server_name() {
        if !sni.eq_ignore_ascii_case(&host) {
            ctx.emit(&host, "", Decision::Deny(DenyReason::HostMismatch), 0);
            return;
        }
    }
    let route = Route {
        host,
        scheme: Scheme::Https,
        port: target.port,
        addrs,
    };
    relay::tunnel_loop(ctx, Buffered::new(tls), Some(up), route).await;
}

/// Absolute-form `http://` requests, possibly several on one connection and
/// for different hosts: each one goes through the whole policy.
async fn plain(ctx: &Ctx, mut guest: Buffered<MuxStream>, first: http::RequestHead) {
    let mut route: Option<Route> = None;
    let mut up: Option<relay::Upstream> = None;
    let mut head = first;
    loop {
        let target = match parse_proxy_request(&head.method, &head.target) {
            Ok(t) => t,
            Err(r) => {
                refuse(ctx, &mut guest.io, raw_host(&head.target, r), "", r).await;
                return;
            }
        };
        if route.as_ref().map(|r| r.host.as_str()) != Some(target.host.as_str()) {
            up = None;
            match ctx
                .policy
                .authorize_target(&*ctx.resolver, target.clone())
                .await
            {
                Ok(a) => {
                    route = Some(Route {
                        host: a.target.host,
                        scheme: Scheme::Http,
                        port: a.target.port,
                        addrs: a.addrs,
                    })
                }
                Err(r) => {
                    refuse(ctx, &mut guest.io, &target.host, &target.path, r).await;
                    return;
                }
            }
        }
        let Some(r) = route.as_ref() else {
            return;
        };
        if relay::exchange(ctx, &mut guest, &mut up, r, head, &target.origin).await == Flow::Close {
            let _ = guest.io.shutdown().await;
            return;
        }
        head = match guest.read_request_head(IDLE).await {
            Ok(Some(h)) => h,
            _ => {
                let _ = guest.io.shutdown().await;
                return;
            }
        };
    }
}

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

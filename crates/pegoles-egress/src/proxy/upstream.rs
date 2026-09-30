//! Upstream connections: connect to the checked address, then TLS verified
//! with `webpki-roots` only.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::http::Buffered;
use super::Ctx;
use crate::policy::{DenyReason, Scheme};

/// TCP connect budget (all addresses together).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// TLS handshake budget, each side.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

pub(crate) type BoxedIo = Box<dyn AsyncStream>;
pub(crate) type ConnectFuture<'a> = Pin<Box<dyn Future<Output = io::Result<BoxedIo>> + Send + 'a>>;

/// How the proxy reaches the addresses the policy checked. Only the
/// production connector exists outside test builds.
pub(crate) trait Connector: Send + Sync {
    fn connect<'a>(&'a self, addrs: &'a [SocketAddr]) -> ConnectFuture<'a>;
}

pub(crate) struct TcpConnector;

impl Connector for TcpConnector {
    fn connect<'a>(&'a self, addrs: &'a [SocketAddr]) -> ConnectFuture<'a> {
        Box::pin(async move {
            let attempt = async {
                let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
                for addr in addrs {
                    match TcpStream::connect(addr).await {
                        Ok(s) => {
                            let _ = s.set_nodelay(true);
                            return Ok(Box::new(s) as BoxedIo);
                        }
                        Err(e) => last = e,
                    }
                }
                Err(last)
            };
            tokio::time::timeout(CONNECT_TIMEOUT, attempt)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))?
        })
    }
}

/// Connection to `host` over `scheme`, to one of the checked `addrs`.
pub(super) async fn connect(
    ctx: &Ctx,
    scheme: Scheme,
    host: &str,
    addrs: &[SocketAddr],
) -> Result<Buffered<BoxedIo>, DenyReason> {
    let tcp = ctx
        .connector
        .connect(addrs)
        .await
        .map_err(|_| DenyReason::UpstreamConnect)?;
    match scheme {
        Scheme::Http => Ok(Buffered::new(tcp)),
        Scheme::Https => {
            let name =
                ServerName::try_from(host.to_string()).map_err(|_| DenyReason::InvalidHost)?;
            let connector = TlsConnector::from(ctx.upstream_tls.clone());
            // A failed verification is a deny, never a click-through.
            let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(name, tcp))
                .await
                .map_err(|_| DenyReason::UpstreamTls)?
                .map_err(|_| DenyReason::UpstreamTls)?;
            Ok(Buffered::new(Box::new(tls) as BoxedIo))
        }
    }
}

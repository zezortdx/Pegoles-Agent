//! Public entry point: one egress session over one byte stream.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::audit::AuditSink;
use crate::mux::{self, Handler, HandshakeError, Running, SessionEnd};
use crate::policy::{Mode, Policy, PolicyError, Resolver, SystemResolver};
use crate::proxy::{self, Budget, Connector, Ctx, TcpConnector, SESSION_BYTE_CAP};
use crate::tls::{self, SessionCa, TlsError};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// `Mode::Off` (and any invalid mode or failed threat snapshot) never
    /// opens the channel.
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
}

/// A running egress session. Dropping it kills every stream and therefore
/// every upstream connection (kill switch).
pub struct EgressSession {
    running: Running,
}

impl EgressSession {
    /// Starts the session over `stream` (the bridge to the guest's egress
    /// channel): builds the policy for `mode` (refuses `Mode::Off`, verifies
    /// the threat snapshot), generates the session CA, sends HELLO with it,
    /// waits for HELLO_ACK, then serves the guest's streams until shutdown.
    ///
    /// Every decision is delivered to `audit`, which must not block.
    pub async fn start<S>(
        stream: S,
        mode: Mode,
        audit: AuditSink,
    ) -> Result<EgressSession, StartError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let policy = Policy::new(mode)?;
        let upstream_tls = tls::upstream_config()?;
        Self::start_with(
            stream,
            policy,
            Arc::new(SystemResolver),
            Arc::new(TcpConnector),
            upstream_tls,
            audit,
        )
        .await
    }

    pub(crate) async fn start_with<S>(
        stream: S,
        policy: Policy,
        resolver: Arc<dyn Resolver>,
        connector: Arc<dyn Connector>,
        upstream_tls: Arc<rustls::ClientConfig>,
        audit: AuditSink,
    ) -> Result<EgressSession, StartError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ca = SessionCa::generate()?;
        let ca_der = ca.ca_der().to_vec();
        let ctx = Arc::new(Ctx {
            policy,
            resolver,
            connector,
            ca,
            upstream_tls,
            audit,
            budget: Budget::new(SESSION_BYTE_CAP),
        });
        let handler: Handler = Arc::new(move |s| Box::pin(proxy::serve(ctx.clone(), s)));
        let running = mux::start_host(stream, ca_der, handler).await?;
        Ok(EgressSession { running })
    }

    /// True once the session has ended (channel closed, protocol violation,
    /// shutdown).
    pub fn is_closed(&self) -> bool {
        self.running.is_closed()
    }

    /// Resolves when the session ends, with the reason.
    pub async fn closed(&self) -> SessionEnd {
        self.running.closed().await
    }

    /// Ends the session: closes every stream and upstream connection.
    pub async fn shutdown(self) {
        self.running.shutdown().await;
    }
}

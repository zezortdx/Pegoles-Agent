//! Test doubles: a fake guest speaking the mux, a local upstream server
//! (TLS or plain), a test PKI. Compiled in test builds only.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pegoles_egress_proto::{Conn, Decoder, Frame, Role};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::mux::{Core, MuxStream};
use crate::policy::{ResolveFuture, Resolver};
use crate::proxy::{BoxedIo, ConnectFuture, Connector};

const IDLE: Duration = Duration::from_secs(10);

// ---- fake guest ---------------------------------------------------------

/// The guest end of the mux: handshakes, opens streams.
pub(crate) struct FakeGuest {
    pub core: Arc<Core>,
    pub ca_der: Vec<u8>,
    tasks: Vec<JoinHandle<()>>,
}

impl FakeGuest {
    pub async fn connect(io: DuplexStream) -> FakeGuest {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let core = Core::new(Conn::new(Role::Guest), out_tx);
        let (mut rd, mut wr) = tokio::io::split(io);
        let writer = tokio::spawn(async move {
            while let Some(b) = out_rx.recv().await {
                if wr.write_all(&b).await.is_err() || wr.flush().await.is_err() {
                    break;
                }
            }
        });
        let (ca_tx, ca_rx) = oneshot::channel();
        let core2 = core.clone();
        let reader = tokio::spawn(async move {
            let mut ca_tx = Some(ca_tx);
            let mut dec = Decoder::new();
            let mut buf = vec![0u8; 16 * 1024];
            'outer: loop {
                let n = match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut input = &buf[..n];
                loop {
                    match dec.decode(&mut input) {
                        Ok(Some(frame)) => {
                            let hello = matches!(frame, Frame::Hello { .. });
                            if let (Frame::Hello { ca_der, .. }, Some(tx)) = (&frame, ca_tx.take())
                            {
                                let _ = tx.send(ca_der.clone());
                            }
                            if core2.on_frame(frame).is_err() {
                                break 'outer;
                            }
                            if hello {
                                if let Ok(ack) = core2.guest_ack() {
                                    core2.send(&ack);
                                }
                            }
                        }
                        Ok(None) if input.is_empty() => break,
                        Ok(None) => {}
                        Err(_) => break 'outer,
                    }
                }
            }
            core2.kill();
        });
        let ca_der = ca_rx.await.expect("the host sends HELLO");
        FakeGuest {
            core,
            ca_der,
            tasks: vec![writer, reader],
        }
    }

    pub fn open(&self) -> MuxStream {
        self.core.open_stream().expect("open stream")
    }

    /// TLS client config trusting only the session CA from HELLO.
    pub fn client_config(&self) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(self.ca_der.clone()))
            .expect("session CA is a valid certificate");
        let mut cfg =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(cfg)
    }

    /// Opens a stream, sends CONNECT, expects the status line, and returns
    /// the raw stream (for a TLS handshake) or the refusal status.
    pub async fn connect_tunnel(&self, authority: &str) -> Result<MuxStream, u16> {
        let mut s = self.open();
        let req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
        s.write_all(req.as_bytes()).await.expect("write CONNECT");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let n = s.read(&mut byte).await.expect("read CONNECT reply");
            assert!(n > 0, "closed before the CONNECT reply: {head:?}");
            head.push(byte[0]);
        }
        let status: u16 = String::from_utf8_lossy(&head)[9..12]
            .parse()
            .expect("status");
        if status == 200 {
            Ok(s)
        } else {
            Err(status)
        }
    }

    /// CONNECT then TLS handshake, verifying the leaf against the session CA.
    pub async fn tls_tunnel(
        &self,
        host: &str,
    ) -> Result<tokio_rustls::client::TlsStream<MuxStream>, u16> {
        let raw = self.connect_tunnel(&format!("{host}:443")).await?;
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).expect("name");
        Ok(TlsConnector::from(self.client_config())
            .connect(name, raw)
            .await
            .expect("TLS handshake with the intercepting proxy"))
    }
}

impl Drop for FakeGuest {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

// ---- resolver / connector ----------------------------------------------------

/// Resolves test names to fixed addresses.
pub(crate) struct FixedResolver(pub Vec<(&'static str, &'static str)>);

impl Resolver for FixedResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        Box::pin(async move {
            self.0
                .iter()
                .find(|(h, _)| *h == host)
                .map(|(_, ip)| vec![ip.parse::<IpAddr>().expect("ip")])
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "nx"))
        })
    }
}

// ---- test PKI ----------------------------------------------------------------

pub(crate) struct TestPki {
    pub root_der: Vec<u8>,
    params: CertificateParams,
    key: KeyPair,
}

impl TestPki {
    pub fn new() -> TestPki {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Pegoles test root");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
        let cert = params.self_signed(&key).expect("root");
        TestPki {
            root_der: cert.der().to_vec(),
            params,
            key,
        }
    }

    /// Server config with a leaf for `names`, signed by this root.
    pub fn server(&self, names: &[&str]) -> Arc<ServerConfig> {
        let params =
            CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .expect("leaf params");
        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).expect("leaf");
        server_config(cert.der().to_vec(), leaf_key.serialize_der())
    }

    /// A self-signed server certificate no root vouches for.
    pub fn untrusted_server(names: &[&str]) -> Arc<ServerConfig> {
        let params =
            CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .expect("params");
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("key");
        let cert = params.self_signed(&key).expect("cert");
        server_config(cert.der().to_vec(), key.serialize_der())
    }
}

fn server_config(cert_der: Vec<u8>, key_der: Vec<u8>) -> Arc<ServerConfig> {
    let mut cfg =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert_der)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
            )
            .expect("server cert");
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(cfg)
}

// ---- local upstream ----------------------------------------------------------------

/// What the upstream saw.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub chunked: bool,
    /// Declare this length instead of `body.len()` (and send only `body`).
    pub declared_length: Option<u64>,
    /// Answer 101 and echo everything afterwards.
    pub websocket_echo: bool,
    /// Read the request, then just wait for the peer to go away.
    pub hang: bool,
    /// Extra bytes written right after the response (a desync attempt).
    pub trailing: Vec<u8>,
}

impl Reply {
    pub fn ok(content_type: &str, body: &[u8]) -> Reply {
        Reply {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.to_vec(),
            ..Reply::default()
        }
    }
}

pub(crate) type HandlerFn = Arc<dyn Fn(&Seen) -> Reply + Send + Sync>;

pub(crate) struct LocalUpstream {
    pub addr: SocketAddr,
    pub connections: Arc<AtomicUsize>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub hang_ended: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Drop for LocalUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalUpstream {
    pub async fn start(tls: Option<Arc<ServerConfig>>, handler: HandlerFn) -> LocalUpstream {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hang_ended = Arc::new(AtomicBool::new(false));
        let (c, s, h) = (connections.clone(), seen.clone(), hang_ended.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                c.fetch_add(1, Ordering::SeqCst);
                let (handler, seen, hang) = (handler.clone(), s.clone(), h.clone());
                let tls = tls.clone();
                tokio::spawn(async move {
                    match tls {
                        Some(cfg) => {
                            if let Ok(t) = TlsAcceptor::from(cfg).accept(tcp).await {
                                serve_conn(t, handler, seen, hang).await;
                            }
                        }
                        None => serve_conn(tcp, handler, seen, hang).await,
                    }
                });
            }
        });
        LocalUpstream {
            addr,
            connections,
            seen,
            hang_ended,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().expect("lock").clone()
    }
}

async fn serve_conn<S>(
    io: S,
    handler: HandlerFn,
    seen: Arc<Mutex<Vec<Seen>>>,
    hang: Arc<AtomicBool>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut b = crate::proxy::testing::Buffered::new(io);
    while let Ok(Some(req)) = b.read_request_head(IDLE).await {
        let body = crate::proxy::testing::read_request_body(&mut b, &req).await;
        let seen_req = Seen {
            method: req.method.clone(),
            target: req.target.clone(),
            headers: req
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name.clone(),
                        String::from_utf8_lossy(&h.value).to_string(),
                    )
                })
                .collect(),
            body,
        };
        if let Ok(mut g) = seen.lock() {
            g.push(seen_req.clone());
        }
        let head_only = seen_req.method == "HEAD";
        let reply = handler(&seen_req);
        if reply.hang {
            let mut buf = [0u8; 256];
            loop {
                match b.io.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            hang.store(true, Ordering::SeqCst);
            return;
        }
        let mut out = format!(
            "HTTP/1.1 {} X\r\n",
            if reply.status == 0 { 200 } else { reply.status }
        );
        for (n, v) in &reply.headers {
            out.push_str(&format!("{n}: {v}\r\n"));
        }
        if reply.websocket_echo {
            out.push_str("Connection: Upgrade\r\nUpgrade: websocket\r\n\r\n");
            let mut io = b.io;
            let _ = io
                .write_all(
                    out.replace("HTTP/1.1 200 X", "HTTP/1.1 101 Switching Protocols")
                        .as_bytes(),
                )
                .await;
            let mut buf = [0u8; 1024];
            while let Ok(n) = io.read(&mut buf).await {
                if n == 0 || io.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            return;
        }
        if reply.chunked {
            out.push_str("Transfer-Encoding: chunked\r\n\r\n");
        } else {
            let len = reply.declared_length.unwrap_or(reply.body.len() as u64);
            out.push_str(&format!("Content-Length: {len}\r\n\r\n"));
        }
        let mut bytes = out.into_bytes();
        if head_only {
            // no body on the wire
        } else if reply.chunked {
            for chunk in reply.body.chunks(7) {
                bytes.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                bytes.extend_from_slice(chunk);
                bytes.extend_from_slice(b"\r\n");
            }
            bytes.extend_from_slice(b"0\r\n\r\n");
        } else {
            bytes.extend_from_slice(&reply.body);
        }
        bytes.extend_from_slice(&reply.trailing);
        if b.io.write_all(&bytes).await.is_err() || b.io.flush().await.is_err() {
            return;
        }
        if reply
            .declared_length
            .is_some_and(|l| l != reply.body.len() as u64)
        {
            return;
        }
    }
}

/// Connector that sends every checked address to a local listener chosen by
/// the IP the resolver returned.
pub(crate) struct LocalConnector(pub Vec<(IpAddr, SocketAddr)>);

impl Connector for LocalConnector {
    fn connect<'a>(&'a self, addrs: &'a [SocketAddr]) -> ConnectFuture<'a> {
        Box::pin(async move {
            for a in addrs {
                if let Some((_, local)) = self.0.iter().find(|(ip, _)| *ip == a.ip()) {
                    let s = tokio::net::TcpStream::connect(local).await?;
                    return Ok(Box::new(s) as BoxedIo);
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "no local upstream for these addresses",
            ))
        })
    }
}

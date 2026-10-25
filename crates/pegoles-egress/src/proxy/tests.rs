//! End-to-end: a fake guest speaks the mux over `tokio::io::duplex`, the
//! session serves it, and a local upstream (TLS or plain) answers. Test
//! builds trust an extra test root upstream; that override does not exist
//! in any other build.

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

use super::testing::{read_response, Buffered, Response};
use crate::audit::{self, AuditEvent};
use crate::mux::SessionEnd;
use crate::policy::{Decision, DenyReason, Mode, Policy, PolicyError};
use crate::session::{EgressSession, StartError};
use crate::testutil::{
    FakeGuest, FixedResolver, HandlerFn, LocalConnector, LocalUpstream, Reply, Seen, TestPki,
};
use crate::threat::ThreatDb;
use crate::tls;

const TLS_IP: &str = "93.184.216.34";
const PLAIN_IP: &str = "93.184.216.40";
const UNTRUSTED_IP: &str = "93.184.216.41";

fn exe_bytes() -> Vec<u8> {
    let mut v = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00".to_vec();
    v.resize(0x80, 0);
    v.extend_from_slice(b"PE\0\0");
    v.resize(4096, 0x90);
    v
}

fn pdf_bytes(len: usize) -> Vec<u8> {
    let mut v = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    v.extend((0..len).map(|i| (i % 251) as u8));
    v.extend_from_slice(b"\n%%EOF\n");
    v
}

fn page(len: usize) -> Vec<u8> {
    (0..len).map(|i| b'a' + (i % 26) as u8).collect()
}

fn handler() -> HandlerFn {
    Arc::new(|r: &Seen| {
        let path = r.target.split('?').next().unwrap_or("").to_string();
        match path.as_str() {
            "/hello" => Reply::ok("text/html", b"hello"),
            "/big" => Reply::ok("text/html", &page(5 * 1024 * 1024)),
            "/setup.exe" => Reply::ok("application/octet-stream", &exe_bytes()),
            "/fake.pdf" => Reply::ok("application/pdf", &exe_bytes()),
            "/fake.png" => Reply::ok("image/png", &exe_bytes()),
            "/doc.pdf" => {
                let mut r = Reply::ok("application/pdf", &pdf_bytes(100_000));
                r.headers.push((
                    "Content-Disposition".into(),
                    "attachment; filename=\"a.pdf\"".into(),
                ));
                r
            }
            "/img.png" => Reply::ok("image/png", b"\x89PNG\r\n\x1a\n0000"),
            "/archive.zip" => Reply::ok("application/zip", b"PK\x03\x04rest of a zip"),
            "/page-zip" => Reply::ok("text/html", b"PK\x03\x04 pretending to be a page"),
            "/chunked" => Reply {
                chunked: true,
                ..Reply::ok("text/html", b"hello chunked world, in tiny pieces")
            },
            "/huge.pdf" => Reply {
                declared_length: Some(60_000_000),
                ..Reply::ok("application/pdf", &pdf_bytes(10))
            },
            "/gz" => {
                let mut r = Reply::ok("text/html", b"\x1f\x8b\x08\x00whatever");
                r.headers.push(("Content-Encoding".into(), "gzip".into()));
                r
            }
            "/dup-ct" => {
                let mut r = Reply::ok("text/html", b"hello");
                r.headers
                    .push(("Content-Type".into(), "application/octet-stream".into()));
                r
            }
            "/dup-cd" => {
                let mut r = Reply::ok("text/html", b"hello");
                r.headers
                    .push(("Content-Disposition".into(), "inline".into()));
                r.headers
                    .push(("Content-Disposition".into(), "attachment".into()));
                r
            }
            "/dup-ce" => {
                let mut r = Reply::ok("text/html", b"\x1f\x8b\x08\x00whatever");
                r.headers
                    .push(("Content-Encoding".into(), "identity".into()));
                r.headers.push(("Content-Encoding".into(), "gzip".into()));
                r
            }
            "/trailing" => Reply {
                trailing:
                    b"HTTP/1.1 200 X\r\nContent-Type: text/html\r\nContent-Length: 6\r\n\r\nPOISON"
                        .to_vec(),
                ..Reply::ok("text/html", b"hello")
            },
            "/echo" => Reply::ok("text/plain", &r.body),
            "/ws" => Reply {
                status: 101,
                websocket_echo: true,
                ..Reply::default()
            },
            "/hang" => Reply {
                hang: true,
                ..Reply::default()
            },
            "/evil/payload.js" => Reply::ok("application/javascript", b"alert(1)"),
            _ => Reply {
                status: 404,
                ..Reply::ok("text/html", b"nope")
            },
        }
    })
}

struct Harness {
    guest: FakeGuest,
    session: EgressSession,
    events: UnboundedReceiver<AuditEvent>,
    tls_up: LocalUpstream,
    plain_up: LocalUpstream,
}

async fn harness(mode: Mode) -> Harness {
    let pki = TestPki::new();
    let names = ["allowed.test", "other.test", "blocked.test", "adult.test"];
    let tls_up = LocalUpstream::start(Some(pki.server(&names)), handler()).await;
    let plain_up = LocalUpstream::start(None, handler()).await;
    let untrusted_up = LocalUpstream::start(
        Some(TestPki::untrusted_server(&["untrusted.test"])),
        handler(),
    )
    .await;
    let resolver = FixedResolver(vec![
        ("allowed.test", TLS_IP),
        ("other.test", TLS_IP),
        ("blocked.test", TLS_IP),
        ("adult.test", TLS_IP),
        ("plain.test", PLAIN_IP),
        ("untrusted.test", UNTRUSTED_IP),
        ("rebind.test", "10.0.0.5"),
    ]);
    let ip = |s: &str| s.parse::<IpAddr>().expect("ip");
    let connector = LocalConnector(vec![
        (ip(TLS_IP), tls_up.addr),
        (ip(PLAIN_IP), plain_up.addr),
        (ip(UNTRUSTED_IP), untrusted_up.addr),
    ]);
    // keep the untrusted upstream alive for the test's duration
    std::mem::forget(untrusted_up);
    let threat = ThreatDb::from_text(
        "blocked.test\n",
        "",
        "allowed.test/evil/payload.js\n",
        "adult.test\n",
        "",
    );
    let policy = Policy::with_threat(mode, threat).expect("policy");
    let (sink, events) = audit::channel();
    let (host_io, guest_io) = tokio::io::duplex(1024 * 1024);
    let (session, guest) = tokio::join!(
        EgressSession::start_with(
            host_io,
            policy,
            Arc::new(resolver),
            Arc::new(connector),
            tls::upstream_config_with_root(&pki.root_der).expect("upstream tls"),
            sink,
        ),
        FakeGuest::connect(guest_io)
    );
    Harness {
        guest,
        session: session.expect("session starts"),
        events,
        tls_up,
        plain_up,
    }
}

fn allow(domains: &[&str]) -> Mode {
    Mode::allowlist(domains.iter().copied()).expect("allowlist")
}

impl Harness {
    async fn event(&mut self, pred: impl Fn(&AuditEvent) -> bool) -> AuditEvent {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let e = self.events.recv().await.expect("audit channel open");
                if pred(&e) {
                    return e;
                }
            }
        })
        .await
        .expect("expected audit event")
    }

    async fn denied_with(&mut self, reason: DenyReason) -> AuditEvent {
        self.event(|e| e.decision == Decision::Deny(reason)).await
    }

    async fn tunnel(
        &self,
        host: &str,
    ) -> Buffered<tokio_rustls::client::TlsStream<crate::mux::MuxStream>> {
        Buffered::new(self.guest.tls_tunnel(host).await.expect("tunnel"))
    }
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(b: &mut Buffered<S>, raw: &str) {
    b.io.write_all(raw.as_bytes()).await.expect("send");
    b.io.flush().await.expect("flush");
}

async fn get<S: AsyncRead + AsyncWrite + Unpin>(
    b: &mut Buffered<S>,
    host: &str,
    path: &str,
) -> Response {
    send(b, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await;
    read_response(b, false).await
}

fn text(r: &Response) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

// ---- allowed traffic ----------------------------------------------------------

#[tokio::test]
async fn allowed_get_works_through_the_interception() {
    let mut h = harness(allow(&["allowed.test"])).await;
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "allowed.test", "/hello?q=1").await;
    assert_eq!(r.head.status, 200);
    assert_eq!(text(&r), "hello");
    assert_eq!(r.header("content-length").as_deref(), Some("5"));

    let seen = h.tls_up.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].target, "/hello?q=1",
        "the upstream still gets the query"
    );
    assert_eq!(seen[0].header("host"), Some("allowed.test"));
    assert_eq!(seen[0].header("accept-encoding"), Some("identity"));

    let connect = h
        .event(|e| e.path.is_empty() && e.decision == Decision::Allow)
        .await;
    assert_eq!(connect.host, "allowed.test");
    assert_eq!(connect.mode, crate::policy::ModeKind::Allowlist);
    let req = h.event(|e| e.path == "/hello").await;
    assert_eq!(req.decision, Decision::Allow);
    assert_eq!(req.bytes, 5);
    assert_eq!(req.reason_code(), "allowed");
    assert!(!format!("{req:?}").contains("q=1"), "no query in the audit");
}

#[tokio::test]
async fn keep_alive_reuses_the_tunnel_and_the_upstream_connection() {
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    for _ in 0..3 {
        let r = get(&mut c, "allowed.test", "/hello").await;
        assert_eq!(text(&r), "hello");
    }
    let r = get(&mut c, "allowed.test", "/chunked").await;
    assert_eq!(r.header("transfer-encoding").as_deref(), Some("chunked"));
    assert_eq!(text(&r), "hello chunked world, in tiny pieces");
    assert_eq!(h.tls_up.connections.load(Ordering::SeqCst), 1);
    assert_eq!(h.tls_up.requests().len(), 4);
}

#[tokio::test]
async fn a_large_page_streams_intact() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "allowed.test", "/big").await;
    assert_eq!(r.body, page(5 * 1024 * 1024));
    let e = h.event(|e| e.path == "/big").await;
    assert_eq!(e.bytes, 5 * 1024 * 1024);
}

#[tokio::test]
async fn head_requests_and_404s_pass_through() {
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    send(&mut c, "HEAD /hello HTTP/1.1\r\nHost: allowed.test\r\n\r\n").await;
    let r = read_response(&mut c, true).await;
    assert_eq!(r.head.status, 200);
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-length").as_deref(), Some("5"));
    let r = get(&mut c, "allowed.test", "/missing").await;
    assert_eq!(r.head.status, 404);
}

#[tokio::test]
async fn request_bodies_are_forwarded_in_both_framings() {
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    send(
        &mut c,
        "POST /echo HTTP/1.1\r\nHost: allowed.test\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello",
    )
    .await;
    assert_eq!(text(&read_response(&mut c, false).await), "hello");
    send(
        &mut c,
        "POST /echo HTTP/1.1\r\nHost: allowed.test\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n",
    )
    .await;
    assert_eq!(text(&read_response(&mut c, false).await), "abcdefg");
    let seen = h.tls_up.requests();
    assert_eq!(seen[0].body, b"hello");
    assert_eq!(seen[1].body, b"abcdefg");
}

#[tokio::test]
async fn proxy_only_headers_are_not_forwarded_but_cookies_are() {
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    send(
        &mut c,
        "GET /hello HTTP/1.1\r\nHost: allowed.test\r\nProxy-Authorization: Basic eDp5\r\nProxy-Connection: keep-alive\r\nCookie: a=b\r\nAccept-Encoding: gzip, br\r\nExpect: 100-continue\r\n\r\n",
    )
    .await;
    assert_eq!(text(&read_response(&mut c, false).await), "hello");
    let seen = &h.tls_up.requests()[0];
    assert_eq!(seen.header("cookie"), Some("a=b"));
    assert_eq!(seen.header("accept-encoding"), Some("identity"));
    assert!(seen.header("proxy-authorization").is_none());
    assert!(seen.header("proxy-connection").is_none());
    assert!(seen.header("expect").is_none());
}

#[tokio::test]
async fn plain_http_requests_go_through_the_same_policy() {
    let mut h = harness(Mode::OpenWeb).await;
    let stream = h.guest.open();
    let mut c = Buffered::new(stream);
    send(&mut c, "GET http://plain.test/hello?x=1 HTTP/1.1\r\nHost: plain.test\r\nProxy-Connection: keep-alive\r\n\r\n").await;
    let r = read_response(&mut c, false).await;
    assert_eq!((r.head.status, text(&r)), (200, "hello".to_string()));
    send(
        &mut c,
        "GET http://plain.test/chunked HTTP/1.1\r\nHost: plain.test\r\n\r\n",
    )
    .await;
    assert_eq!(
        text(&read_response(&mut c, false).await),
        "hello chunked world, in tiny pieces"
    );
    let seen = h.plain_up.requests();
    assert_eq!(seen[0].target, "/hello?x=1");
    assert_eq!(seen[0].header("host"), Some("plain.test"));
    assert_eq!(h.plain_up.connections.load(Ordering::SeqCst), 1);
    let e = h
        .event(|e| e.path == "/hello" && e.host == "plain.test")
        .await;
    assert_eq!(e.decision, Decision::Allow);
    // a blocked download over plain http
    send(
        &mut c,
        "GET http://plain.test/setup.exe HTTP/1.1\r\nHost: plain.test\r\n\r\n",
    )
    .await;
    let r = read_response(&mut c, false).await;
    assert_eq!(r.head.status, 403);
    h.denied_with(DenyReason::BlockedSignature).await;
}

#[tokio::test]
async fn websocket_is_passed_through_after_the_policy_allowed_the_host() {
    let h = harness(allow(&["allowed.test"])).await;
    let mut c = h.tunnel("allowed.test").await;
    send(
        &mut c,
        "GET /ws HTTP/1.1\r\nHost: allowed.test\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
    )
    .await;
    let head = c
        .read_response_head(Duration::from_secs(5))
        .await
        .expect("101");
    assert_eq!(head.status, 101);
    c.io.write_all(b"ping-pong").await.expect("write");
    c.io.flush().await.expect("flush");
    let mut got = Vec::new();
    while got.len() < 9 {
        if c.unread().is_empty() {
            let n = c.fill(Duration::from_secs(5)).await.expect("read");
            assert!(n > 0);
        }
        let n = c.unread().len();
        got.extend_from_slice(c.unread());
        c.consume(n);
    }
    assert_eq!(got, b"ping-pong");
    let seen = &h.tls_up.requests()[0];
    assert_eq!(seen.header("upgrade"), Some("websocket"));
}

#[tokio::test]
async fn an_unrequested_101_is_refused() {
    // The upstream answers 101 to a plain GET: the proxy must not turn it
    // into a raw pipe.
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    send(&mut c, "GET /ws HTTP/1.1\r\nHost: allowed.test\r\n\r\n").await;
    let r = c
        .read_response_head(Duration::from_secs(5))
        .await
        .expect("head");
    assert_eq!(r.status, 502);
}

// ---- denied traffic --------------------------------------------------------------

#[tokio::test]
async fn a_host_outside_the_allowlist_gets_403_and_no_upstream_contact() {
    let mut h = harness(allow(&["allowed.test"])).await;
    assert_eq!(
        h.guest.connect_tunnel("other.test:443").await.err(),
        Some(403)
    );
    let e = h.denied_with(DenyReason::NotInAllowlist).await;
    assert_eq!(e.host, "other.test");
    assert_eq!(h.tls_up.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn always_deny_beats_the_allowlist() {
    let mut h = harness(allow(&["blocked.test", "allowed.test"])).await;
    assert_eq!(
        h.guest.connect_tunnel("blocked.test:443").await.err(),
        Some(403)
    );
    h.denied_with(DenyReason::ThreatMalware).await;
    assert_eq!(h.tls_up.connections.load(Ordering::SeqCst), 0);
    // a threat URL on an allowed host is cut per request inside the tunnel
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "allowed.test", "/evil/payload.js?v=2").await;
    assert_eq!(r.head.status, 403);
    h.denied_with(DenyReason::ThreatUrl).await;
    assert!(h.tls_up.requests().is_empty());
}

#[tokio::test]
async fn categories_block_in_open_web_only() {
    let mut h = harness(Mode::OpenWeb).await;
    assert_eq!(
        h.guest.connect_tunnel("adult.test:443").await.err(),
        Some(403)
    );
    h.denied_with(DenyReason::CategoryAdult).await;
    let h2 = harness(allow(&["adult.test"])).await;
    let mut c = h2.tunnel("adult.test").await;
    assert_eq!(get(&mut c, "adult.test", "/hello").await.head.status, 200);
}

#[tokio::test]
async fn dns_rebinding_to_a_private_address_is_denied() {
    let mut h = harness(Mode::OpenWeb).await;
    assert_eq!(
        h.guest.connect_tunnel("rebind.test:443").await.err(),
        Some(403)
    );
    h.denied_with(DenyReason::NonGlobalAddress).await;
}

#[tokio::test]
async fn transport_rule_violations_are_refused() {
    let mut h = harness(Mode::OpenWeb).await;
    for (authority, reason) in [
        ("93.184.216.34:443", DenyReason::IpLiteral),
        ("[2001:db8::1]:443", DenyReason::IpLiteral),
        ("allowed.test:8443", DenyReason::PortNotAllowed),
        ("allowed.test:80", DenyReason::PortNotAllowed),
        ("localhost:443", DenyReason::ReservedName),
        ("printer.local:443", DenyReason::ReservedName),
        ("intranet:443", DenyReason::NoDot),
        ("аpple.com:443", DenyReason::MixedScript),
        ("user@allowed.test:443", DenyReason::Userinfo),
    ] {
        assert_eq!(
            h.guest.connect_tunnel(authority).await.err(),
            Some(403),
            "{authority}"
        );
        h.denied_with(reason).await;
    }
    assert_eq!(h.tls_up.connections.load(Ordering::SeqCst), 0);

    for (raw, status, reason) in [
        (
            "TRACE http://plain.test/ HTTP/1.1\r\nHost: plain.test\r\n\r\n",
            403,
            DenyReason::MethodNotAllowed,
        ),
        (
            "GET http://127.0.0.1/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            403,
            DenyReason::IpLiteral,
        ),
        (
            "GET http://plain.test:8080/ HTTP/1.1\r\nHost: plain.test\r\n\r\n",
            403,
            DenyReason::PortNotAllowed,
        ),
        (
            "GET https://plain.test/ HTTP/1.1\r\nHost: plain.test\r\n\r\n",
            403,
            DenyReason::SchemeNotAllowed,
        ),
        (
            "GET /origin-form HTTP/1.1\r\nHost: plain.test\r\n\r\n",
            400,
            DenyReason::MalformedRequest,
        ),
        (
            "GET http://u:p@plain.test/ HTTP/1.1\r\nHost: plain.test\r\n\r\n",
            403,
            DenyReason::Userinfo,
        ),
    ] {
        let mut c = Buffered::new(h.guest.open());
        send(&mut c, raw).await;
        let head = c
            .read_response_head(Duration::from_secs(5))
            .await
            .expect("head");
        assert_eq!(head.status, status, "{raw}");
        h.denied_with(reason).await;
    }
    assert_eq!(h.plain_up.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn oversized_and_garbage_request_heads_are_refused() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = Buffered::new(h.guest.open());
    let long = format!(
        "GET http://plain.test/{} HTTP/1.1\r\n\r\n",
        "a".repeat(9000)
    );
    send(&mut c, &long).await;
    let head = c
        .read_response_head(Duration::from_secs(5))
        .await
        .expect("head");
    assert_eq!(head.status, 431);
    h.denied_with(DenyReason::HeadTooLarge).await;

    let mut c = Buffered::new(h.guest.open());
    c.io.write_all(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n")
        .await
        .expect("send");
    let head = c
        .read_response_head(Duration::from_secs(5))
        .await
        .expect("head");
    assert_eq!(head.status, 400);
    h.denied_with(DenyReason::MalformedRequest).await;
}

#[tokio::test]
async fn a_host_header_other_than_the_connect_host_is_refused() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "other.test", "/hello").await;
    assert_eq!(r.head.status, 403);
    h.denied_with(DenyReason::HostMismatch).await;
    assert!(h.tls_up.requests().is_empty());
}

#[tokio::test]
async fn smuggling_shapes_and_oversized_bodies_are_refused() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    send(
        &mut c,
        "POST /echo HTTP/1.1\r\nHost: allowed.test\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    )
    .await;
    assert_eq!(read_response(&mut c, false).await.head.status, 400);
    h.denied_with(DenyReason::MalformedRequest).await;

    let mut c = h.tunnel("allowed.test").await;
    send(
        &mut c,
        "POST /echo HTTP/1.1\r\nHost: allowed.test\r\nContent-Length: 10485761\r\n\r\n",
    )
    .await;
    assert_eq!(read_response(&mut c, false).await.head.status, 413);
    h.denied_with(DenyReason::BodyTooLarge).await;
    assert!(h.tls_up.requests().is_empty());
}

#[tokio::test]
async fn an_upstream_with_an_untrusted_certificate_is_a_deny_not_a_click_through() {
    let mut h = harness(Mode::OpenWeb).await;
    assert_eq!(
        h.guest.connect_tunnel("untrusted.test:443").await.err(),
        Some(502)
    );
    h.denied_with(DenyReason::UpstreamTls).await;
}

// ---- response inspection ----------------------------------------------------------

#[tokio::test]
async fn an_exe_download_is_blocked() {
    let mut h = harness(allow(&["allowed.test"])).await;
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "allowed.test", "/setup.exe").await;
    assert_eq!(r.head.status, 403);
    assert!(!r.body.starts_with(b"MZ"));
    assert!(text(&r).contains("blocked by Pegoles"));
    let e = h.denied_with(DenyReason::BlockedSignature).await;
    assert_eq!(e.path, "/setup.exe");
    // the connection closed after the block
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), c.io.read_to_end(&mut rest)).await;
    assert!(rest.is_empty());
}

#[tokio::test]
async fn a_pdf_download_passes_byte_for_byte() {
    let mut h = harness(allow(&["allowed.test"])).await;
    let mut c = h.tunnel("allowed.test").await;
    let r = get(&mut c, "allowed.test", "/doc.pdf").await;
    assert_eq!(r.head.status, 200);
    assert_eq!(r.body, pdf_bytes(100_000));
    assert_eq!(
        r.header("content-disposition").as_deref(),
        Some("attachment; filename=\"a.pdf\"")
    );
    let e = h.event(|e| e.path == "/doc.pdf").await;
    assert_eq!(e.decision, Decision::Allow);
    // and the tunnel stays usable after an allowed download
    assert_eq!(text(&get(&mut c, "allowed.test", "/hello").await), "hello");
}

#[tokio::test]
async fn an_exe_disguised_as_a_pdf_or_an_image_is_blocked() {
    let mut h = harness(allow(&["allowed.test"])).await;
    for path in ["/fake.pdf", "/fake.png"] {
        let mut c = h.tunnel("allowed.test").await;
        let r = get(&mut c, "allowed.test", path).await;
        assert_eq!(r.head.status, 403, "{path}");
        assert!(!r.body.starts_with(b"MZ"), "{path}");
        let e = h
            .event(|e| e.path == path && e.decision != Decision::Allow)
            .await;
        assert_eq!(
            e.decision,
            Decision::Deny(DenyReason::BlockedSignature),
            "{path}"
        );
    }
}

#[tokio::test]
async fn archives_and_signature_bearing_pages_are_blocked_images_pass() {
    let h = harness(Mode::OpenWeb).await;
    for path in ["/archive.zip", "/page-zip"] {
        let mut c = h.tunnel("allowed.test").await;
        assert_eq!(
            get(&mut c, "allowed.test", path).await.head.status,
            403,
            "{path}"
        );
    }
    let mut c = h.tunnel("allowed.test").await;
    assert_eq!(
        get(&mut c, "allowed.test", "/img.png").await.head.status,
        200
    );
}

#[tokio::test]
async fn content_encoded_bodies_cannot_be_inspected_and_are_blocked() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    assert_eq!(get(&mut c, "allowed.test", "/gz").await.head.status, 403);
    h.denied_with(DenyReason::EncodedBody).await;
}

#[tokio::test]
async fn duplicate_type_disposition_or_encoding_headers_fail_closed() {
    let mut h = harness(Mode::OpenWeb).await;
    for path in ["/dup-ct", "/dup-cd", "/dup-ce"] {
        let mut c = h.tunnel("allowed.test").await;
        assert_eq!(
            get(&mut c, "allowed.test", path).await.head.status,
            502,
            "{path}"
        );
        let e = h
            .event(|e| e.path == path && e.decision != Decision::Allow)
            .await;
        assert_eq!(
            e.decision,
            Decision::Deny(DenyReason::UpstreamProtocol),
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_upstream_connection_with_unread_bytes_is_not_reused() {
    let h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    assert_eq!(
        text(&get(&mut c, "allowed.test", "/trailing").await),
        "hello"
    );
    // The injected "POISON" response must never answer the next request.
    assert_eq!(text(&get(&mut c, "allowed.test", "/hello").await), "hello");
    assert_eq!(h.tls_up.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn refused_requests_never_audit_userinfo() {
    let mut h = harness(Mode::OpenWeb).await;
    for raw in [
        "GET http://user:secret@evil.com/ HTTP/1.1\r\nHost: evil.com\r\n\r\n",
        "GET http://secret@evil.com/ HTTP/1.1\r\nHost: evil.com\r\n\r\n",
        "GET http://user:se/secret@evil.com/ HTTP/1.1\r\nHost: evil.com\r\n\r\n",
        "GET http://user:secret@evil.com:99999/ HTTP/1.1\r\nHost: evil.com\r\n\r\n",
        "CONNECT user:secret@evil.com:443 HTTP/1.1\r\nHost: evil.com:443\r\n\r\n",
    ] {
        let mut c = Buffered::new(h.guest.open());
        send(&mut c, raw).await;
        let _ = c.read_response_head(Duration::from_secs(5)).await;
        let e = h.event(|e| e.decision != Decision::Allow).await;
        assert!(
            !format!("{e:?}").contains("secret"),
            "audit leaked userinfo for {raw:?}: {e:?}"
        );
    }
}

#[tokio::test]
async fn a_download_declared_over_50_mib_is_blocked_before_its_body() {
    let mut h = harness(Mode::OpenWeb).await;
    let mut c = h.tunnel("allowed.test").await;
    assert_eq!(
        get(&mut c, "allowed.test", "/huge.pdf").await.head.status,
        403
    );
    h.denied_with(DenyReason::DownloadTooLarge).await;
}

// ---- lifecycle ----------------------------------------------------------------------

#[tokio::test]
async fn off_mode_refuses_to_start_and_never_touches_the_stream() {
    let (host_io, mut guest_io) = tokio::io::duplex(4096);
    let (sink, _rx) = audit::channel();
    let err = EgressSession::start(host_io, Mode::Off, sink)
        .await
        .err()
        .expect("Off must not start");
    assert!(
        matches!(err, StartError::Policy(PolicyError::ModeOff)),
        "{err}"
    );
    let mut buf = [0u8; 8];
    // the host end was dropped without writing HELLO
    assert_eq!(guest_io.read(&mut buf).await.expect("eof"), 0);
}

#[tokio::test]
async fn the_public_start_works_end_to_end_with_the_builtin_snapshot() {
    let (host_io, guest_io) = tokio::io::duplex(64 * 1024);
    let (sink, _rx) = audit::channel();
    let mode = Mode::allowlist(["example.com"]).expect("list");
    let (session, guest) = tokio::join!(
        EgressSession::start(host_io, mode, sink),
        FakeGuest::connect(guest_io)
    );
    let session = session.expect("starts");
    assert!(!guest.ca_der.is_empty());
    // not allowlisted: refused without any network access
    assert_eq!(
        guest.connect_tunnel("wikipedia.org:443").await.err(),
        Some(403)
    );
    session.shutdown().await;
}

#[tokio::test]
async fn dropping_the_handle_kills_the_upstream_connections() {
    let h = harness(Mode::OpenWeb).await;
    let Harness {
        guest,
        session,
        tls_up,
        ..
    } = h;
    let mut c = Buffered::new(guest.tls_tunnel("allowed.test").await.expect("tunnel"));
    send(&mut c, "GET /hang HTTP/1.1\r\nHost: allowed.test\r\n\r\n").await;
    for _ in 0..100 {
        if !tls_up.requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        tls_up.requests().len(),
        1,
        "the request reached the upstream"
    );
    assert!(!tls_up.hang_ended.load(Ordering::SeqCst));
    drop(session);
    for _ in 0..100 {
        if tls_up.hang_ended.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the upstream connection stayed open after the handle was dropped");
}

#[tokio::test]
async fn shutdown_closes_the_session_with_a_reason() {
    let h = harness(Mode::OpenWeb).await;
    assert!(!h.session.is_closed());
    let Harness { guest, session, .. } = h;
    drop(guest);
    let end = tokio::time::timeout(Duration::from_secs(5), session.closed())
        .await
        .expect("ends when the guest goes away");
    assert_eq!(end, SessionEnd::PeerClosed);
    session.shutdown().await;
}

#[test]
fn the_session_byte_budget_is_enforced() {
    let b = super::Budget::new(100);
    assert!(b.charge(60));
    assert!(b.charge(40));
    assert!(!b.charge(1));
    assert_eq!(super::SESSION_BYTE_CAP, 1 << 30);
}

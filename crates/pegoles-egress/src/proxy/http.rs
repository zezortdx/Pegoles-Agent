//! Bounded HTTP/1.1 head parsing and message framing.
//!
//! Heads are parsed with `httparse` inside hard limits (request line 8 KiB,
//! head 32 KiB, 100 headers) and re-serialized by the proxy, so nothing the
//! guest or an upstream sends is forwarded verbatim. Ambiguous framing
//! (both `Content-Length` and `Transfer-Encoding`, differing lengths,
//! unknown transfer codings) is refused, which closes the request-smuggling
//! door.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::policy::DenyReason;

pub const MAX_HEAD: usize = 32 * 1024;
pub const MAX_REQUEST_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 100;
pub const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug)]
pub enum HttpError {
    Io,
    Timeout,
    /// The peer closed in the middle of a message.
    Eof,
    HeadTooLarge,
    Malformed,
}

impl From<io::Error> for HttpError {
    fn from(_: io::Error) -> Self {
        HttpError::Io
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestHead {
    pub method: String,
    pub target: String,
    pub minor: u8,
    pub headers: Vec<Header>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseHead {
    pub minor: u8,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<Header>,
}

/// Read buffer over a stream. Reads are cancel safe (`read_buf`).
pub struct Buffered<S> {
    pub io: S,
    buf: Vec<u8>,
    pos: usize,
}

impl<S: AsyncRead + Unpin> Buffered<S> {
    pub fn new(io: S) -> Self {
        Buffered {
            io,
            buf: Vec::new(),
            pos: 0,
        }
    }

    pub fn unread(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    pub fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.buf.len());
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
    }

    /// Reads more bytes (at least one unless EOF, which returns 0).
    pub async fn fill(&mut self, idle: Duration) -> Result<usize, HttpError> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.reserve(READ_CHUNK);
        match tokio::time::timeout(idle, self.io.read_buf(&mut self.buf)).await {
            Err(_) => Err(HttpError::Timeout),
            Ok(Err(_)) => Err(HttpError::Io),
            Ok(Ok(n)) => Ok(n),
        }
    }

    /// Next request head; `None` on a clean close before any byte.
    pub async fn read_request_head(
        &mut self,
        idle: Duration,
    ) -> Result<Option<RequestHead>, HttpError> {
        loop {
            if !self.unread().is_empty() {
                let unread = self.unread();
                if unread.len() >= MAX_REQUEST_LINE && !unread[..MAX_REQUEST_LINE].contains(&b'\n')
                {
                    return Err(HttpError::HeadTooLarge);
                }
                let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
                let mut req = httparse::Request::new(&mut headers);
                match req.parse(unread) {
                    Ok(httparse::Status::Complete(n)) => {
                        if n > MAX_HEAD {
                            return Err(HttpError::HeadTooLarge);
                        }
                        let head = RequestHead {
                            method: req.method.ok_or(HttpError::Malformed)?.to_string(),
                            target: req.path.ok_or(HttpError::Malformed)?.to_string(),
                            minor: req.version.ok_or(HttpError::Malformed)?,
                            headers: collect(req.headers),
                        };
                        self.consume(n);
                        return Ok(Some(head));
                    }
                    Ok(httparse::Status::Partial) => {
                        if unread.len() >= MAX_HEAD {
                            return Err(HttpError::HeadTooLarge);
                        }
                    }
                    Err(httparse::Error::TooManyHeaders) => return Err(HttpError::HeadTooLarge),
                    Err(_) => return Err(HttpError::Malformed),
                }
            }
            if self.fill(idle).await? == 0 {
                return if self.unread().is_empty() {
                    Ok(None)
                } else {
                    Err(HttpError::Eof)
                };
            }
        }
    }

    pub async fn read_response_head(&mut self, idle: Duration) -> Result<ResponseHead, HttpError> {
        loop {
            if !self.unread().is_empty() {
                let unread = self.unread();
                let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
                let mut resp = httparse::Response::new(&mut headers);
                match resp.parse(unread) {
                    Ok(httparse::Status::Complete(n)) => {
                        if n > MAX_HEAD {
                            return Err(HttpError::HeadTooLarge);
                        }
                        let head = ResponseHead {
                            minor: resp.version.ok_or(HttpError::Malformed)?,
                            status: resp.code.ok_or(HttpError::Malformed)?,
                            reason: sanitize_reason(resp.reason.unwrap_or("")),
                            headers: collect(resp.headers),
                        };
                        self.consume(n);
                        return Ok(head);
                    }
                    Ok(httparse::Status::Partial) => {
                        if unread.len() >= MAX_HEAD {
                            return Err(HttpError::HeadTooLarge);
                        }
                    }
                    Err(httparse::Error::TooManyHeaders) => return Err(HttpError::HeadTooLarge),
                    Err(_) => return Err(HttpError::Malformed),
                }
            }
            if self.fill(idle).await? == 0 {
                return Err(HttpError::Eof);
            }
        }
    }
}

fn collect(headers: &[httparse::Header<'_>]) -> Vec<Header> {
    headers
        .iter()
        .map(|h| Header {
            name: h.name.to_string(),
            value: h.value.to_vec(),
        })
        .collect()
}

/// Reason phrases are re-emitted: printable ASCII only, bounded.
fn sanitize_reason(r: &str) -> String {
    r.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(64)
        .collect()
}

// ---- header helpers ----------------------------------------------------

pub fn find<'a>(headers: &'a [Header], name: &'a str) -> impl Iterator<Item = &'a Header> + 'a {
    headers
        .iter()
        .filter(move |h| h.name.eq_ignore_ascii_case(name))
}

/// Value of a header that may appear at most once.
pub fn single<'a>(headers: &'a [Header], name: &'a str) -> Result<Option<&'a [u8]>, HttpError> {
    let mut it = find(headers, name);
    let first = it.next();
    if it.next().is_some() {
        return Err(HttpError::Malformed);
    }
    Ok(first.map(|h| h.value.as_slice()))
}

/// Any comma-separated token of any `name` header equals `token`.
pub fn has_token(headers: &[Header], name: &str, token: &str) -> bool {
    find(headers, name).any(|h| {
        String::from_utf8_lossy(&h.value)
            .split(',')
            .any(|t| t.trim().eq_ignore_ascii_case(token))
    })
}

/// The connection will not be reused after this message.
pub fn wants_close(minor: u8, headers: &[Header]) -> bool {
    has_token(headers, "connection", "close")
        || (minor == 0 && !has_token(headers, "connection", "keep-alive"))
}

fn content_length(headers: &[Header]) -> Result<Option<u64>, HttpError> {
    let mut found: Option<u64> = None;
    for h in find(headers, "content-length") {
        let s = std::str::from_utf8(&h.value).map_err(|_| HttpError::Malformed)?;
        let s = s.trim();
        if s.is_empty() || s.len() > 19 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HttpError::Malformed);
        }
        let n: u64 = s.parse().map_err(|_| HttpError::Malformed)?;
        match found {
            Some(prev) if prev != n => return Err(HttpError::Malformed),
            _ => found = Some(n),
        }
    }
    Ok(found)
}

fn is_chunked_only(headers: &[Header]) -> Result<bool, HttpError> {
    let mut it = find(headers, "transfer-encoding");
    let Some(first) = it.next() else {
        return Ok(false);
    };
    if it.next().is_some() {
        return Err(HttpError::Malformed);
    }
    let v = String::from_utf8_lossy(&first.value);
    if v.trim().eq_ignore_ascii_case("chunked") {
        Ok(true)
    } else {
        Err(HttpError::Malformed)
    }
}

/// How a message body is delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    None,
    Length(u64),
    Chunked,
    /// Response only: body runs until the server closes.
    UntilClose,
}

/// Request framing. A body over `max_body` is `BodyTooLarge`.
pub fn request_framing(h: &RequestHead, max_body: u64) -> Result<Framing, DenyReason> {
    let bad = |_| DenyReason::MalformedRequest;
    let chunked = is_chunked_only(&h.headers).map_err(bad)?;
    let len = content_length(&h.headers).map_err(bad)?;
    if chunked {
        if len.is_some() || h.minor == 0 {
            return Err(DenyReason::MalformedRequest);
        }
        return Ok(Framing::Chunked);
    }
    match len {
        Some(0) | None => Ok(Framing::None),
        Some(n) if n > max_body => Err(DenyReason::BodyTooLarge),
        Some(n) => Ok(Framing::Length(n)),
    }
}

/// Response framing (RFC 9112 section 6.3, strictly).
pub fn response_framing(head_request: bool, h: &ResponseHead) -> Result<Framing, HttpError> {
    if head_request || (100..200).contains(&h.status) || h.status == 204 || h.status == 304 {
        return Ok(Framing::None);
    }
    let chunked = is_chunked_only(&h.headers)?;
    let len = content_length(&h.headers)?;
    if chunked {
        if len.is_some() || h.minor == 0 {
            return Err(HttpError::Malformed);
        }
        return Ok(Framing::Chunked);
    }
    match len {
        Some(n) => Ok(Framing::Length(n)),
        None => Ok(Framing::UntilClose),
    }
}

// ---- serialization -----------------------------------------------------

/// Headers never forwarded: hop-by-hop, framing (re-derived), proxy-only.
const DROPPED: &[&str] = &[
    "connection",
    "proxy-connection",
    "proxy-authorization",
    "proxy-authenticate",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "content-length",
    "upgrade",
    "expect",
    "host",
    "accept-encoding",
];

fn push_header(out: &mut Vec<u8>, name: &str, value: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");
}

fn forwarded(headers: &[Header]) -> impl Iterator<Item = &Header> {
    headers
        .iter()
        .filter(|h| !DROPPED.iter().any(|d| h.name.eq_ignore_ascii_case(d)))
}

/// Request line and headers for the upstream. `host` is the canonical
/// `Host` value; `Accept-Encoding: identity` makes bodies inspectable.
pub fn serialize_request(
    req: &RequestHead,
    origin_target: &str,
    host: &str,
    framing: Framing,
    websocket: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(req.method.as_bytes());
    out.push(b' ');
    out.extend_from_slice(origin_target.as_bytes());
    out.extend_from_slice(b" HTTP/1.1\r\n");
    push_header(&mut out, "Host", host.as_bytes());
    push_header(&mut out, "Accept-Encoding", b"identity");
    for h in forwarded(&req.headers) {
        push_header(&mut out, &h.name, &h.value);
    }
    match framing {
        Framing::None | Framing::UntilClose => {
            // Methods that carry a body declare an empty one explicitly.
            if matches!(req.method.as_str(), "POST" | "PUT" | "PATCH") {
                push_header(&mut out, "Content-Length", b"0");
            }
        }
        Framing::Length(n) => push_header(&mut out, "Content-Length", n.to_string().as_bytes()),
        Framing::Chunked => push_header(&mut out, "Transfer-Encoding", b"chunked"),
    }
    if websocket {
        push_header(&mut out, "Connection", b"Upgrade");
        push_header(&mut out, "Upgrade", b"websocket");
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// Status line and headers for the guest. `close`: announce that the
/// connection closes after this response. `keep_length`: for bodiless
/// responses, keep the upstream `Content-Length` (HEAD, 304).
pub fn serialize_response(
    resp: &ResponseHead,
    framing: Framing,
    close: bool,
    websocket_101: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", resp.status, resp.reason).as_bytes());
    for h in forwarded(&resp.headers) {
        push_header(&mut out, &h.name, &h.value);
    }
    match framing {
        Framing::None => {
            if let Ok(Some(v)) = single(&resp.headers, "content-length") {
                push_header(&mut out, "Content-Length", v);
            }
        }
        Framing::Length(n) => push_header(&mut out, "Content-Length", n.to_string().as_bytes()),
        Framing::Chunked => push_header(&mut out, "Transfer-Encoding", b"chunked"),
        Framing::UntilClose => {}
    }
    if websocket_101 {
        push_header(&mut out, "Connection", b"Upgrade");
        push_header(&mut out, "Upgrade", b"websocket");
    } else if close || framing == Framing::UntilClose {
        push_header(&mut out, "Connection", b"close");
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// `Host` header value for a target: port only when not the scheme default.
pub fn host_header(host: &str, port: u16, default_port: u16) -> String {
    if port == default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

/// True for a well-formed WebSocket upgrade request.
pub fn is_websocket_request(req: &RequestHead) -> bool {
    req.method == "GET"
        && req.minor == 1
        && has_token(&req.headers, "connection", "upgrade")
        && find(&req.headers, "upgrade").count() == 1
        && has_token(&req.headers, "upgrade", "websocket")
        && find(&req.headers, "upgrade").all(|h| {
            String::from_utf8_lossy(&h.value)
                .trim()
                .eq_ignore_ascii_case("websocket")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(raw: &str) -> RequestHead {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut r = httparse::Request::new(&mut headers);
        match r.parse(raw.as_bytes()).expect("parse") {
            httparse::Status::Complete(_) => {}
            httparse::Status::Partial => panic!("partial"),
        }
        RequestHead {
            method: r.method.expect("m").into(),
            target: r.path.expect("p").into(),
            minor: r.version.expect("v"),
            headers: collect(r.headers),
        }
    }

    fn resp(raw: &str) -> ResponseHead {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut r = httparse::Response::new(&mut headers);
        r.parse(raw.as_bytes()).expect("parse");
        ResponseHead {
            minor: r.version.expect("v"),
            status: r.code.expect("c"),
            reason: r.reason.unwrap_or("").into(),
            headers: collect(r.headers),
        }
    }

    #[test]
    fn request_framing_rules() {
        let f = |raw: &str| request_framing(&req(raw), 1000);
        assert_eq!(f("GET / HTTP/1.1\r\nHost: a\r\n\r\n"), Ok(Framing::None));
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n"),
            Ok(Framing::Length(5))
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
            Ok(Framing::None)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Ok(Framing::Chunked)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\ntransfer-encoding: Chunked\r\n\r\n"),
            Ok(Framing::Chunked)
        );
        // smuggling shapes
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n"),
            Ok(Framing::Length(5))
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nTransfer-Encoding: gzip, chunked\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 1, 2\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 99999999999999999999\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        assert_eq!(
            f("POST / HTTP/1.0\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(DenyReason::MalformedRequest)
        );
        // body cap
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 1001\r\n\r\n"),
            Err(DenyReason::BodyTooLarge)
        );
        assert_eq!(
            f("POST / HTTP/1.1\r\nContent-Length: 1000\r\n\r\n"),
            Ok(Framing::Length(1000))
        );
    }

    #[test]
    fn response_framing_rules() {
        let f = |head: bool, raw: &str| response_framing(head, &resp(raw)).ok();
        assert_eq!(
            f(false, "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n"),
            Some(Framing::Length(3))
        );
        assert_eq!(
            f(
                false,
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Some(Framing::Chunked)
        );
        assert_eq!(
            f(false, "HTTP/1.1 200 OK\r\n\r\n"),
            Some(Framing::UntilClose)
        );
        assert_eq!(
            f(true, "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n"),
            Some(Framing::None)
        );
        assert_eq!(
            f(false, "HTTP/1.1 204 No Content\r\n\r\n"),
            Some(Framing::None)
        );
        assert_eq!(
            f(false, "HTTP/1.1 304 Not Modified\r\n\r\n"),
            Some(Framing::None)
        );
        assert_eq!(
            f(
                false,
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            None
        );
        assert_eq!(
            f(false, "HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n"),
            None
        );
    }

    #[test]
    fn serialized_request_is_rebuilt_not_forwarded() {
        let r = req(
            "POST /p?q=1 HTTP/1.1\r\nHost: evil.example\r\nConnection: keep-alive, X-Hop\r\n\
Proxy-Authorization: Basic eA==\r\nAccept-Encoding: gzip, br\r\nExpect: 100-continue\r\n\
Content-Length: 4\r\nCookie: a=b\r\nUpgrade: h2c\r\nX-Custom: 1\r\n\r\n",
        );
        let framing = request_framing(&r, 100).expect("framing");
        let out = String::from_utf8(serialize_request(
            &r,
            "/p?q=1",
            "good.example",
            framing,
            false,
        ))
        .expect("utf8");
        assert!(out.starts_with(
            "POST /p?q=1 HTTP/1.1\r\nHost: good.example\r\nAccept-Encoding: identity\r\n"
        ));
        assert!(out.contains("Cookie: a=b\r\n"));
        assert!(out.contains("X-Custom: 1\r\n"));
        assert!(out.contains("Content-Length: 4\r\n"));
        for gone in [
            "evil.example",
            "Proxy-Authorization",
            "gzip",
            "Expect",
            "h2c",
            "keep-alive",
        ] {
            assert!(!out.contains(gone), "{gone} leaked: {out}");
        }
        assert!(out.ends_with("\r\n\r\n"));
    }

    #[test]
    fn serialized_response_reframes() {
        let r = resp("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\nKeep-Alive: timeout=5\r\nSet-Cookie: a=b\r\n\r\n");
        let out = String::from_utf8(serialize_response(&r, Framing::Chunked, false, false))
            .expect("utf8");
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(out.contains("Content-Type: text/html\r\n"));
        assert!(out.contains("Set-Cookie: a=b\r\n"));
        assert!(out.contains("Transfer-Encoding: chunked\r\n"));
        assert!(!out.contains("Keep-Alive"));
        assert!(!out.contains("keep-alive"));
        let out = String::from_utf8(serialize_response(&r, Framing::UntilClose, false, false))
            .expect("utf8");
        assert!(out.contains("Connection: close\r\n"));
        let head = resp("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n");
        let out = String::from_utf8(serialize_response(&head, Framing::None, false, false))
            .expect("utf8");
        assert!(
            out.contains("Content-Length: 10\r\n"),
            "HEAD keeps the declared length"
        );
    }

    #[test]
    fn connection_persistence() {
        let h = |raw: &str| req(raw);
        let r = h("GET / HTTP/1.1\r\n\r\n");
        assert!(!wants_close(r.minor, &r.headers));
        let r = h("GET / HTTP/1.1\r\nConnection: close\r\n\r\n");
        assert!(wants_close(r.minor, &r.headers));
        let r = h("GET / HTTP/1.0\r\n\r\n");
        assert!(wants_close(r.minor, &r.headers));
        let r = h("GET / HTTP/1.0\r\nConnection: Keep-Alive\r\n\r\n");
        assert!(!wants_close(r.minor, &r.headers));
    }

    #[test]
    fn websocket_detection_is_strict() {
        let ok = req("GET /ws HTTP/1.1\r\nHost: a\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket\r\n\r\n");
        assert!(is_websocket_request(&ok));
        assert!(!is_websocket_request(&req(
            "GET /ws HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: h2c\r\n\r\n"
        )));
        assert!(!is_websocket_request(&req(
            "POST /ws HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
        )));
        assert!(!is_websocket_request(&req(
            "GET /ws HTTP/1.1\r\nUpgrade: websocket\r\n\r\n"
        )));
    }

    #[tokio::test]
    async fn head_limits_are_enforced() {
        let idle = Duration::from_secs(1);
        // request line over 8 KiB
        let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(9000));
        let mut b = Buffered::new(long.as_bytes());
        assert!(matches!(
            b.read_request_head(idle).await,
            Err(HttpError::HeadTooLarge)
        ));
        // head over 32 KiB
        let mut big = String::from("GET / HTTP/1.1\r\n");
        for i in 0..90 {
            big.push_str(&format!("X-{i}: {}\r\n", "v".repeat(400)));
        }
        big.push_str("\r\n");
        let mut b = Buffered::new(big.as_bytes());
        assert!(matches!(
            b.read_request_head(idle).await,
            Err(HttpError::HeadTooLarge)
        ));
        // too many headers
        let mut many = String::from("GET / HTTP/1.1\r\n");
        for i in 0..150 {
            many.push_str(&format!("X-{i}: v\r\n"));
        }
        many.push_str("\r\n");
        let mut b = Buffered::new(many.as_bytes());
        assert!(matches!(
            b.read_request_head(idle).await,
            Err(HttpError::HeadTooLarge)
        ));
        // garbage
        let mut b = Buffered::new(&b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n"[..]);
        assert!(matches!(
            b.read_request_head(idle).await,
            Err(HttpError::Malformed)
        ));
        // clean EOF and truncation
        let mut b = Buffered::new(&b""[..]);
        assert!(matches!(b.read_request_head(idle).await, Ok(None)));
        let mut b = Buffered::new(&b"GET / HT"[..]);
        assert!(matches!(
            b.read_request_head(idle).await,
            Err(HttpError::Eof)
        ));
    }

    #[tokio::test]
    async fn pipelined_heads_are_split_correctly() {
        let idle = Duration::from_secs(1);
        let raw = b"GET /a HTTP/1.1\r\nHost: x\r\n\r\nGET /b HTTP/1.1\r\nHost: x\r\n\r\n";
        let mut b = Buffered::new(&raw[..]);
        assert_eq!(
            b.read_request_head(idle)
                .await
                .expect("ok")
                .expect("head")
                .target,
            "/a"
        );
        assert_eq!(
            b.read_request_head(idle)
                .await
                .expect("ok")
                .expect("head")
                .target,
            "/b"
        );
        assert!(matches!(b.read_request_head(idle).await, Ok(None)));
    }
}

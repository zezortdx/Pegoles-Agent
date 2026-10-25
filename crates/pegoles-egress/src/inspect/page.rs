//! Static pages the proxy answers with. Nothing in them depends on guest,
//! upstream or model data.

use crate::policy::DenyReason;

/// The static page a blocked response is replaced with.
pub const BLOCK_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Blocked</title></head><body><h1>Blocked</h1>\
<p>This content was blocked by Pegoles to keep the computer safe.</p></body></html>";

const ERROR_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Unavailable</title></head><body><h1>Unavailable</h1>\
<p>Pegoles could not complete this request.</p></body></html>";

fn response(status: u16, phrase: &str, body: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {phrase}\r\nContent-Type: text/html; charset=utf-8\r\n\
Content-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Full HTTP/1.1 response (head and body) for a blocked response. Closes
/// the connection: whatever followed on the wire is not trusted.
pub fn blocked_response() -> Vec<u8> {
    response(403, "Forbidden", BLOCK_PAGE)
}

/// Static answer for a refused or failed request.
pub fn refusal_response(reason: DenyReason) -> Vec<u8> {
    match reason {
        DenyReason::ResolveFailed
        | DenyReason::UpstreamConnect
        | DenyReason::UpstreamTls
        | DenyReason::UpstreamProtocol => response(502, "Bad Gateway", ERROR_PAGE),
        DenyReason::HeadTooLarge => response(431, "Request Header Fields Too Large", ERROR_PAGE),
        DenyReason::BodyTooLarge => response(413, "Content Too Large", ERROR_PAGE),
        DenyReason::MalformedRequest => response(400, "Bad Request", ERROR_PAGE),
        DenyReason::IdleTimeout => response(408, "Request Timeout", ERROR_PAGE),
        _ => blocked_response(),
    }
}

//! Response inspection (EGRESS.md section 5), applied after TLS
//! interception to every response.
//!
//! - Executable or archive signatures are blocked whatever the declared type.
//! - A download (`Content-Disposition: attachment`, or a type outside the
//!   web-page set) passes only if it is a safe type **and** its magic bytes
//!   agree: PDF, PNG, JPEG, GIF, WebP, plain text, CSV.
//! - Web-page resources pass if not attachments and not caught above.
//! - Downloads over 50 MiB are blocked.
//!
//! The proxy buffers the body head, calls [`inspect_head`], and only then
//! releases the response to the guest.

mod magic;
mod page;

pub use magic::{looks_like_text, sniff_blocked, sniff_safe, Blocked, Safe};
pub use page::{blocked_response, refusal_response, BLOCK_PAGE};

use crate::policy::DenyReason;

/// Largest download that may pass.
pub const MAX_DOWNLOAD: u64 = 50 * 1024 * 1024;
/// Head bytes buffered for downloads: covers the ISO 9660 identifier at
/// offset 0x8001 and a useful text sample.
pub const HEAD_WINDOW: usize = 33 * 1024;
/// Head bytes buffered for web-page resources before release (every
/// signature that matters for them sits at offset 0).
pub const PAGE_PROBE: usize = 16;

/// The parts of a response the decision needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseMeta {
    /// Lowercase media type without parameters, if declared.
    pub content_type: Option<String>,
    pub attachment: bool,
    pub content_length: Option<u64>,
    /// `Content-Encoding` other than identity: the wire bytes are not the
    /// payload and cannot be inspected.
    pub encoded: bool,
}

impl ResponseMeta {
    /// Builds the metadata from raw header values.
    pub fn from_headers(
        content_type: Option<&[u8]>,
        content_disposition: Option<&[u8]>,
        content_length: Option<u64>,
        content_encoding: Option<&[u8]>,
    ) -> ResponseMeta {
        ResponseMeta {
            content_type: content_type.and_then(media_type),
            attachment: content_disposition.is_some_and(is_attachment),
            content_length,
            encoded: content_encoding.is_some_and(|v| {
                let v = String::from_utf8_lossy(v);
                let v = v.trim();
                !(v.is_empty() || v.eq_ignore_ascii_case("identity"))
            }),
        }
    }
}

fn media_type(value: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(value).ok()?;
    let main = s.split(';').next()?.trim().to_ascii_lowercase();
    if main.is_empty() {
        None
    } else {
        Some(main)
    }
}

fn is_attachment(value: &[u8]) -> bool {
    let s = String::from_utf8_lossy(value);
    s.split(';')
        .next()
        .is_some_and(|d| d.trim().eq_ignore_ascii_case("attachment"))
}

/// Types a page loads as sub-resources or renders in place.
pub fn is_page_type(media: &str) -> bool {
    let (top, sub) = media.split_once('/').unwrap_or((media, ""));
    match top {
        "image" | "audio" | "video" | "font" => true,
        "text" => matches!(
            sub,
            "html" | "css" | "javascript" | "ecmascript" | "json" | "xml" | "event-stream"
        ),
        "application" => {
            matches!(
                sub,
                "xhtml+xml"
                    | "javascript"
                    | "x-javascript"
                    | "ecmascript"
                    | "json"
                    | "xml"
                    | "wasm"
                    | "x-ndjson"
                    | "manifest+json"
                    | "font-woff"
                    | "font-woff2"
                    | "vnd.ms-fontobject"
            ) || sub.ends_with("+json")
                || sub.ends_with("+xml")
                || sub.starts_with("x-font-")
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Page,
    Download,
}

pub fn classify(meta: &ResponseMeta) -> Class {
    match (&meta.content_type, meta.attachment) {
        (_, true) => Class::Download,
        (Some(t), false) if is_page_type(t) => Class::Page,
        // No declared type is decided on the content (see `inspect_head`).
        (None, false) => Class::Page,
        (Some(_), false) => Class::Download,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block(DenyReason),
}

/// Decision possible before any body byte: encoded bodies, and downloads
/// whose declared length is over the cap.
pub fn precheck(meta: &ResponseMeta) -> Verdict {
    if meta.encoded {
        return Verdict::Block(DenyReason::EncodedBody);
    }
    if classify(meta) == Class::Download && meta.content_length.is_some_and(|n| n > MAX_DOWNLOAD) {
        return Verdict::Block(DenyReason::DownloadTooLarge);
    }
    Verdict::Allow
}

/// Body bytes to buffer before [`inspect_head`] can decide.
pub fn head_needed(meta: &ResponseMeta) -> usize {
    match classify(meta) {
        Class::Download => HEAD_WINDOW,
        Class::Page if meta.content_type.is_none() => HEAD_WINDOW,
        Class::Page => PAGE_PROBE,
    }
}

/// Byte cap to enforce while streaming (`None`: only the session cap).
pub fn stream_cap(meta: &ResponseMeta) -> Option<u64> {
    (classify(meta) == Class::Download).then_some(MAX_DOWNLOAD)
}

/// Decides on the body head. `complete`: the slice is the whole body.
pub fn inspect_head(meta: &ResponseMeta, head: &[u8], complete: bool) -> Verdict {
    if let v @ Verdict::Block(_) = precheck(meta) {
        return v;
    }
    if sniff_blocked(head, complete).is_some() {
        return Verdict::Block(DenyReason::BlockedSignature);
    }
    let safe_ok = match (classify(meta), meta.content_type.as_deref()) {
        (Class::Page, Some(_)) => return Verdict::Allow,
        // Undeclared type: a recognised safe format or plain text passes.
        (Class::Page, None) => sniff_safe(head).is_some() || looks_like_text(head),
        (Class::Download, Some(t)) => safe_agrees(t, head),
        (Class::Download, None) => sniff_safe(head).is_some() || looks_like_text(head),
    };
    if safe_ok {
        Verdict::Allow
    } else {
        Verdict::Block(DenyReason::DownloadNotSafe)
    }
}

/// Declared type is a safe type and the magic bytes agree with it.
fn safe_agrees(media: &str, head: &[u8]) -> bool {
    match media {
        "application/pdf" => sniff_safe(head) == Some(Safe::Pdf),
        "image/png" => sniff_safe(head) == Some(Safe::Png),
        "image/jpeg" => sniff_safe(head) == Some(Safe::Jpeg),
        "image/gif" => sniff_safe(head) == Some(Safe::Gif),
        "image/webp" => sniff_safe(head) == Some(Safe::Webp),
        "text/plain" | "text/csv" | "application/csv" => looks_like_text(head),
        _ => false,
    }
}

#[cfg(test)]
mod tests;

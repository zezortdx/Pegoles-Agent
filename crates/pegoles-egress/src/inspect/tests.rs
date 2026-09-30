use super::*;

fn meta(ct: Option<&str>, attachment: bool) -> ResponseMeta {
    ResponseMeta {
        content_type: ct.map(str::to_string),
        attachment,
        content_length: None,
        encoded: false,
    }
}

fn pad(mut v: Vec<u8>) -> Vec<u8> {
    v.resize(v.len().max(64), 0x41);
    v
}

fn with_magic_at(offset: usize, magic: &[u8], total: usize) -> Vec<u8> {
    let mut v = vec![b'a'; total];
    v[offset..offset + magic.len()].copy_from_slice(magic);
    v
}

/// Real file headers.
type Sample = (&'static str, Vec<u8>, Option<Blocked>, Option<Safe>);

fn samples() -> Vec<Sample> {
    let mut pe = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00".to_vec();
    pe.extend_from_slice(&[0; 44]);
    pe.extend_from_slice(b"\x80\x00\x00\x00");
    let mut udif = vec![b'x'; 2048];
    let n = udif.len();
    udif[n - 512..n - 508].copy_from_slice(b"koly");
    vec![
        ("pe", pe, Some(Blocked::Pe), None),
        (
            "elf",
            pad(b"\x7fELF\x02\x01\x01\x00".to_vec()),
            Some(Blocked::Elf),
            None,
        ),
        (
            "macho64",
            pad(b"\xcf\xfa\xed\xfe\x07\x00\x00\x01".to_vec()),
            Some(Blocked::MachO),
            None,
        ),
        (
            "macho32",
            pad(b"\xfe\xed\xfa\xce\x00\x00\x00\x12".to_vec()),
            Some(Blocked::MachO),
            None,
        ),
        (
            "macho-le32",
            pad(b"\xce\xfa\xed\xfe\x07\x00\x00\x00".to_vec()),
            Some(Blocked::MachO),
            None,
        ),
        (
            "macho-fat",
            pad(b"\xca\xfe\xba\xbe\x00\x00\x00\x02".to_vec()),
            Some(Blocked::MachOFat),
            None,
        ),
        (
            "shebang",
            pad(b"#!/bin/sh\necho hi\n".to_vec()),
            Some(Blocked::Shebang),
            None,
        ),
        (
            "zip",
            pad(b"PK\x03\x04\x14\x00\x00\x00\x08\x00".to_vec()),
            Some(Blocked::Zip),
            None,
        ),
        (
            "zip-empty",
            pad(b"PK\x05\x06\x00\x00\x00\x00".to_vec()),
            Some(Blocked::Zip),
            None,
        ),
        (
            "7z",
            pad(b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec()),
            Some(Blocked::SevenZip),
            None,
        ),
        (
            "rar4",
            pad(b"Rar!\x1a\x07\x00\xcf".to_vec()),
            Some(Blocked::Rar),
            None,
        ),
        (
            "rar5",
            pad(b"Rar!\x1a\x07\x01\x00".to_vec()),
            Some(Blocked::Rar),
            None,
        ),
        (
            "gzip",
            pad(b"\x1f\x8b\x08\x00\x00\x00\x00\x00".to_vec()),
            Some(Blocked::Gzip),
            None,
        ),
        (
            "xz",
            pad(b"\xfd7zXZ\x00\x00\x04".to_vec()),
            Some(Blocked::Xz),
            None,
        ),
        (
            "bzip2",
            pad(b"BZh91AY&SY".to_vec()),
            Some(Blocked::Bzip2),
            None,
        ),
        (
            "zstd",
            pad(b"\x28\xb5\x2f\xfd\x04\x58".to_vec()),
            Some(Blocked::Zstd),
            None,
        ),
        (
            "cab",
            pad(b"MSCF\x00\x00\x00\x00".to_vec()),
            Some(Blocked::Cab),
            None,
        ),
        (
            "ole-msi",
            pad(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00\x00".to_vec()),
            Some(Blocked::Ole),
            None,
        ),
        (
            "ar-deb",
            pad(b"!<arch>\ndebian-binary   ".to_vec()),
            Some(Blocked::Ar),
            None,
        ),
        (
            "xar-pkg",
            pad(b"xar!\x00\x1c\x00\x01".to_vec()),
            Some(Blocked::Xar),
            None,
        ),
        (
            "iso",
            with_magic_at(0x8001, b"CD001", 0x8800),
            Some(Blocked::Iso),
            None,
        ),
        (
            "pdf",
            pad(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec()),
            None,
            Some(Safe::Pdf),
        ),
        (
            "png",
            pad(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec()),
            None,
            Some(Safe::Png),
        ),
        (
            "jpeg",
            pad(b"\xff\xd8\xff\xe0\x00\x10JFIF\x00".to_vec()),
            None,
            Some(Safe::Jpeg),
        ),
        (
            "gif87",
            pad(b"GIF87a\x01\x00\x01\x00".to_vec()),
            None,
            Some(Safe::Gif),
        ),
        (
            "gif89",
            pad(b"GIF89a\x01\x00\x01\x00".to_vec()),
            None,
            Some(Safe::Gif),
        ),
        (
            "webp",
            pad(b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec()),
            None,
            Some(Safe::Webp),
        ),
        (
            "wav-not-webp",
            pad(b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec()),
            None,
            None,
        ),
        ("text", pad(b"name,age\nann,3\n".to_vec()), None, None),
        ("html", pad(b"<!doctype html><html>".to_vec()), None, None),
        ("udif-not-blocked-when-partial", udif, None, None),
    ]
}

#[test]
fn magic_table_matches_real_headers() {
    for (name, bytes, blocked, safe) in samples() {
        let partial = name == "udif-not-blocked-when-partial";
        assert_eq!(sniff_blocked(&bytes, false), blocked, "{name}");
        assert_eq!(sniff_safe(&bytes), safe, "{name}");
        if partial {
            assert_eq!(
                sniff_blocked(&bytes, true),
                Some(Blocked::Udif),
                "{name} complete"
            );
        }
    }
}

#[test]
fn udif_needs_the_complete_body() {
    let mut body = vec![0u8; 4096];
    let n = body.len();
    body[n - 512..n - 508].copy_from_slice(b"koly");
    assert_eq!(sniff_blocked(&body, false), None);
    assert_eq!(sniff_blocked(&body, true), Some(Blocked::Udif));
    assert_eq!(sniff_blocked(&[0u8; 100], true), None);
}

#[test]
fn signatures_are_blocked_whatever_the_declared_type() {
    let declared = [
        Some("text/html"),
        Some("application/pdf"),
        Some("image/png"),
        Some("application/javascript"),
        Some("application/octet-stream"),
        Some("text/plain"),
        Some("video/mp4"),
        None,
    ];
    for (name, bytes, blocked, _) in samples() {
        if blocked.is_none() {
            continue;
        }
        for ct in declared {
            for attachment in [false, true] {
                let v = inspect_head(&meta(ct, attachment), &bytes, bytes.len() < 40_000);
                assert_eq!(
                    v,
                    Verdict::Block(DenyReason::BlockedSignature),
                    "{name} as {ct:?} attachment={attachment}"
                );
            }
        }
    }
}

#[test]
fn safe_downloads_pass_only_with_agreeing_magic() {
    const OK: Verdict = Verdict::Allow;
    const NOT_SAFE: Verdict = Verdict::Block(DenyReason::DownloadNotSafe);
    let table: &[(&str, &str, Verdict)] = &[
        ("application/pdf", "pdf", OK),
        ("image/png", "png", OK),
        ("image/jpeg", "jpeg", OK),
        ("image/gif", "gif87", OK),
        ("image/gif", "gif89", OK),
        ("image/webp", "webp", OK),
        ("text/plain", "text", OK),
        ("text/csv", "text", OK),
        // declared safe, content says otherwise
        ("application/pdf", "png", NOT_SAFE),
        ("image/png", "pdf", NOT_SAFE),
        ("image/jpeg", "gif89", NOT_SAFE),
        ("application/pdf", "text", NOT_SAFE),
        ("image/webp", "wav-not-webp", NOT_SAFE),
        ("text/plain", "png", NOT_SAFE),
        // not safe types, even with harmless content
        ("application/octet-stream", "pdf", NOT_SAFE),
        ("application/zip", "pdf", NOT_SAFE),
        ("application/x-msdownload", "text", NOT_SAFE),
        ("application/msword", "pdf", NOT_SAFE),
        ("image/svg+xml", "text", NOT_SAFE),
        ("text/html", "html", NOT_SAFE),
    ];
    let all = samples();
    for (ct, sample, want) in table {
        let bytes = &all.iter().find(|s| s.0 == *sample).expect("sample").1;
        // attachment: always a download
        assert_eq!(
            inspect_head(&meta(Some(ct), true), bytes, true),
            *want,
            "{ct} with {sample} (attachment)"
        );
        // inline: a download only when the type is outside the page set
        if !is_page_type(ct) {
            assert_eq!(
                inspect_head(&meta(Some(ct), false), bytes, true),
                *want,
                "{ct} with {sample} (inline)"
            );
        }
    }
}

#[test]
fn disguised_executable_as_pdf_is_blocked() {
    let (_, pe, _, _) = samples().into_iter().find(|s| s.0 == "pe").expect("pe");
    for attachment in [false, true] {
        let v = inspect_head(&meta(Some("application/pdf"), attachment), &pe, true);
        assert_eq!(v, Verdict::Block(DenyReason::BlockedSignature));
    }
}

#[test]
fn page_resources_pass_unless_signature_or_attachment() {
    let page = b"<!doctype html><html><body>hi</body></html>";
    for ct in [
        "text/html",
        "text/css",
        "text/javascript",
        "application/javascript",
        "application/json",
        "application/ld+json",
        "application/manifest+json",
        "application/xhtml+xml",
        "application/xml",
        "image/svg+xml",
        "image/png",
        "image/avif",
        "audio/mpeg",
        "video/mp4",
        "font/woff2",
        "application/font-woff",
        "application/x-font-ttf",
        "application/wasm",
        "text/event-stream",
    ] {
        assert!(is_page_type(ct), "{ct}");
        assert_eq!(classify(&meta(Some(ct), false)), Class::Page, "{ct}");
        assert_eq!(
            inspect_head(&meta(Some(ct), false), page, true),
            Verdict::Allow,
            "{ct}"
        );
    }
    // an attachment is a download even for a page type
    assert_eq!(
        inspect_head(&meta(Some("text/html"), true), page, true),
        Verdict::Block(DenyReason::DownloadNotSafe)
    );
    assert_eq!(
        inspect_head(
            &meta(Some("image/png"), true),
            b"\x89PNG\r\n\x1a\n0000",
            true
        ),
        Verdict::Allow
    );
}

#[test]
fn types_outside_the_page_set_are_downloads() {
    for ct in [
        "application/octet-stream",
        "application/zip",
        "application/x-msdownload",
        "application/x-protobuf",
        "application/vnd.android.package-archive",
        "application/x-sh",
        "application/msword",
        "binary/octet-stream",
        "text/x-shellscript",
    ] {
        assert!(!is_page_type(ct), "{ct}");
        assert_eq!(classify(&meta(Some(ct), false)), Class::Download, "{ct}");
    }
}

#[test]
fn undeclared_type_is_decided_on_content() {
    let m = meta(None, false);
    assert_eq!(inspect_head(&m, b"<html>ok</html>", true), Verdict::Allow);
    assert_eq!(inspect_head(&m, b"%PDF-1.4 ...", true), Verdict::Allow);
    assert_eq!(
        inspect_head(&m, b"\x00\x01\x02\x03binary\x00", true),
        Verdict::Block(DenyReason::DownloadNotSafe)
    );
    assert_eq!(
        inspect_head(&m, b"MZ\x90\x00", true),
        Verdict::Block(DenyReason::BlockedSignature)
    );
}

#[test]
fn text_detection() {
    assert!(looks_like_text(b""));
    assert!(looks_like_text("héllo wörld\n\ttab\r\n".as_bytes()));
    assert!(looks_like_text(&"日本語".as_bytes()[..7])); // cut inside a character
    assert!(!looks_like_text(b"abc\x00def"));
    assert!(!looks_like_text(b"abc\x01def"));
    assert!(!looks_like_text(b"\xff\xfeh\x00i\x00"));
    assert!(!looks_like_text(b"\x80\x81\x82"));
}

#[test]
fn downloads_over_50_mib_are_blocked_but_pages_are_not_capped_here() {
    let mut m = meta(Some("application/pdf"), false);
    m.content_length = Some(MAX_DOWNLOAD);
    assert_eq!(precheck(&m), Verdict::Allow);
    m.content_length = Some(MAX_DOWNLOAD + 1);
    assert_eq!(precheck(&m), Verdict::Block(DenyReason::DownloadTooLarge));
    assert_eq!(stream_cap(&m), Some(MAX_DOWNLOAD));
    let mut v = meta(Some("video/mp4"), false);
    v.content_length = Some(MAX_DOWNLOAD * 10);
    assert_eq!(precheck(&v), Verdict::Allow);
    assert_eq!(stream_cap(&v), None);
}

#[test]
fn content_encoding_other_than_identity_is_blocked() {
    for enc in ["gzip", "br", "deflate", "zstd", "GZIP"] {
        let m = ResponseMeta::from_headers(Some(b"text/html"), None, None, Some(enc.as_bytes()));
        assert_eq!(
            precheck(&m),
            Verdict::Block(DenyReason::EncodedBody),
            "{enc}"
        );
    }
    for enc in ["identity", "", " Identity "] {
        let m = ResponseMeta::from_headers(Some(b"text/html"), None, None, Some(enc.as_bytes()));
        assert_eq!(precheck(&m), Verdict::Allow, "{enc:?}");
    }
}

#[test]
fn header_parsing() {
    let m = ResponseMeta::from_headers(
        Some(b"Text/HTML; charset=UTF-8"),
        Some(b"Attachment; filename=\"a.exe\""),
        Some(5),
        None,
    );
    assert_eq!(m.content_type.as_deref(), Some("text/html"));
    assert!(m.attachment);
    let m = ResponseMeta::from_headers(Some(b""), Some(b"inline; filename=a.pdf"), None, None);
    assert_eq!(m.content_type, None);
    assert!(!m.attachment);
    let m = ResponseMeta::from_headers(Some(b"\xff\xfe"), Some(b"\xff attachment"), None, None);
    assert_eq!(m.content_type, None);
}

#[test]
fn head_needs() {
    assert_eq!(head_needed(&meta(Some("text/html"), false)), PAGE_PROBE);
    assert_eq!(
        head_needed(&meta(Some("application/pdf"), false)),
        HEAD_WINDOW
    );
    assert_eq!(head_needed(&meta(Some("image/png"), true)), HEAD_WINDOW);
    assert_eq!(head_needed(&meta(None, false)), HEAD_WINDOW);
}

#[test]
fn block_page_is_static_and_well_formed() {
    let r = blocked_response();
    let text = String::from_utf8(r).expect("utf8");
    let (head, body) = text.split_once("\r\n\r\n").expect("head/body");
    assert!(head.starts_with("HTTP/1.1 403 Forbidden"));
    assert!(head.contains(&format!("Content-Length: {}", body.len())));
    assert!(head.contains("Connection: close"));
    assert_eq!(body, BLOCK_PAGE);
    assert!(body.len() < 512);
}

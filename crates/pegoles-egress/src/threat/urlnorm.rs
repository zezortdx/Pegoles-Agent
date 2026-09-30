//! URL key normalization for the malware URL list: both the list entries
//! (at load time) and the request being checked go through [`key`], so
//! `//`, `/./`, `/../`, `;params`, percent-encoded unreserved characters and
//! host case cannot be used to step around an exact match.
//!
//! Only unreserved characters (`A-Z a-z 0-9 - . _ ~`) are decoded: `%2F`
//! and friends stay escaped (they are data, not separators) with their hex
//! digits upper-cased. Paths stay case-sensitive.

/// The lookup key `host/path` for a request.
pub(super) fn key(host: &str, path: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    format!("{host}{}", normalize_path(path))
}

/// A list line `host/path` to its lookup key. Lines without a path (or
/// that are not of that shape) are kept lower-cased.
pub(super) fn normalize_entry(line: &str) -> String {
    match line.find('/') {
        Some(i) => key(&line[..i], &line[i..]),
        None => line.to_ascii_lowercase(),
    }
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Decodes unreserved escapes, upper-cases the others.
fn canon_escapes(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                let v = h * 16 + l;
                if is_unreserved(v) {
                    out.push(char::from(v));
                } else {
                    out.push_str(&format!("%{v:02X}"));
                }
                i += 3;
                continue;
            }
        }
        // `path` is valid UTF-8 and only ASCII bytes are special, so
        // pushing the char at this boundary is exact.
        let ch = path[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `/a//b/./c/../d;x=1` -> `/a/b/d` (leading `/` always, dot segments
/// resolved without leaving the root, empty segments collapsed).
fn normalize_path(path: &str) -> String {
    let decoded = canon_escapes(path);
    let mut stack: Vec<&str> = Vec::new();
    let mut trailing_slash = false;
    for raw in decoded.split('/') {
        let seg = raw.split(';').next().unwrap_or("");
        trailing_slash = false;
        match seg {
            "" => trailing_slash = true,
            "." => trailing_slash = true,
            ".." => {
                stack.pop();
                trailing_slash = true;
            }
            s => stack.push(s),
        }
    }
    let mut out = String::from("/");
    out.push_str(&stack.join("/"));
    if trailing_slash && !stack.is_empty() {
        out.push('/');
    }
    out
}

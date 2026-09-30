//! Host normalization: lowercase, IDNA, mixed-script (homograph) deny.

use std::net::IpAddr;

use super::DenyReason;

/// Longest input accepted before IDNA processing.
const MAX_RAW_LEN: usize = 512;

/// Suffixes that never name a public host (besides `localhost` itself).
const RESERVED_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "lan",
    "home.arpa",
    "onion",
    "arpa",
];

/// Normalizes a host taken from a proxy request or allowlist entry to its
/// ASCII (punycode) lowercase form and applies the host-level transport
/// rules: no IP literals, no userinfo, a dot, no reserved names, no mixed
/// scripts.
pub fn normalize(raw: &str) -> Result<String, DenyReason> {
    if raw.is_empty() || raw.len() > MAX_RAW_LEN {
        return Err(DenyReason::InvalidHost);
    }
    if raw.contains('@') {
        return Err(DenyReason::Userinfo);
    }
    if raw.starts_with('[') || raw.contains(':') {
        return Err(DenyReason::IpLiteral);
    }
    if raw
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || "/\\?#%*".contains(c))
    {
        return Err(DenyReason::InvalidHost);
    }
    let trimmed = raw.strip_suffix('.').unwrap_or(raw);
    if trimmed.is_empty() || trimmed.starts_with('.') || trimmed.ends_with('.') {
        return Err(DenyReason::InvalidHost);
    }
    let ascii = idna::domain_to_ascii_strict(trimmed).map_err(|_| DenyReason::InvalidHost)?;
    if ascii.is_empty() || ascii.len() > 253 {
        return Err(DenyReason::InvalidHost);
    }
    if is_ip_like(&ascii) {
        return Err(DenyReason::IpLiteral);
    }
    if is_reserved(&ascii) {
        return Err(DenyReason::ReservedName);
    }
    if !ascii.contains('.') {
        return Err(DenyReason::NoDot);
    }
    if ascii.contains("xn--") {
        let (unicode, res) = idna::domain_to_unicode(&ascii);
        if res.is_err() || unicode.split('.').any(suspicious_label) {
            return Err(DenyReason::MixedScript);
        }
    }
    Ok(ascii)
}

/// Dotted quads, bare integers and the odd numeric forms resolvers accept
/// (`127.1`, `0x7f.0.0.1`, `2130706433`): a last label that is all digits,
/// or hexadecimal with a `0x` prefix, is an address, not a name.
fn is_ip_like(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    let tld = host.rsplit('.').next().unwrap_or(host);
    if tld.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    tld.strip_prefix("0x")
        .is_some_and(|h| h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_reserved(host: &str) -> bool {
    RESERVED_SUFFIXES
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    /// Digits, hyphen, combining marks, prolonged sound mark: neutral.
    Common,
    Latin,
    Cyrillic,
    Greek,
    Armenian,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Bopomofo,
}

/// `None`: a script Pegoles does not accept in host names (fail closed).
fn script_of(c: char) -> Option<Script> {
    let u = u32::from(c);
    Some(match u {
        0x30..=0x39 | 0x2D => Script::Common,
        0x0300..=0x036F | 0x30FC | 0x200C | 0x200D => Script::Common,
        0x41..=0x5A | 0x61..=0x7A | 0x00C0..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Script::Cyrillic,
        0x0530..=0x058F => Script::Armenian,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF => Script::Arabic,
        0x0900..=0x097F => Script::Devanagari,
        0x0E00..=0x0E7F => Script::Thai,
        0x3040..=0x309F => Script::Hiragana,
        0x30A0..=0x30FF => Script::Katakana,
        0x3100..=0x312F => Script::Bopomofo,
        0x1100..=0x11FF | 0xAC00..=0xD7AF => Script::Hangul,
        0x3005 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2A6DF => {
            Script::Han
        }
        _ => return None,
    })
}

/// Cyrillic and Greek letters that render like a Latin letter.
fn latin_lookalike(c: char) -> bool {
    matches!(
        c,
        'а' | 'с'
            | 'е'
            | 'о'
            | 'р'
            | 'х'
            | 'у'
            | 'і'
            | 'ј'
            | 'ѕ'
            | 'ԁ'
            | 'ӏ'
            | 'ԛ'
            | 'ԝ'
            | 'ɡ'
            | 'ο'
            | 'ν'
            | 'ρ'
            | 'α'
            | 'ι'
            | 'κ'
            | 'τ'
            | 'υ'
    )
}

/// Mixed scripts in one label (outside the Japanese / Korean / Chinese
/// combinations), an unsupported script, or a Cyrillic/Greek label made only
/// of Latin lookalikes (whole-script confusable such as `аррӏе`).
fn suspicious_label(label: &str) -> bool {
    let mut seen: Vec<Script> = Vec::new();
    for c in label.chars() {
        match script_of(c) {
            None => return true,
            Some(Script::Common) => {}
            Some(s) if seen.contains(&s) => {}
            Some(s) => seen.push(s),
        }
    }
    let allowed_combo =
        |set: &[Script], allowed: &[Script]| set.iter().all(|s| allowed.contains(s));
    let multi_ok = seen.len() <= 1
        || allowed_combo(&seen, &[Script::Han, Script::Hiragana, Script::Katakana])
        || allowed_combo(&seen, &[Script::Han, Script::Bopomofo])
        || allowed_combo(&seen, &[Script::Han, Script::Hangul]);
    if !multi_ok {
        return true;
    }
    if seen == [Script::Cyrillic] || seen == [Script::Greek] {
        let letters = label.chars().filter(|c| c.is_alphabetic());
        if letters.clone().count() > 0 && letters.clone().all(latin_lookalike) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_and_strips_one_trailing_dot() {
        assert_eq!(normalize("Example.COM").as_deref(), Ok("example.com"));
        assert_eq!(normalize("example.com.").as_deref(), Ok("example.com"));
        assert_eq!(normalize("example.com.."), Err(DenyReason::InvalidHost));
        assert_eq!(normalize(".example.com"), Err(DenyReason::InvalidHost));
    }

    #[test]
    fn idna_names_become_punycode() {
        assert_eq!(
            normalize("bücher.example").as_deref(),
            Ok("xn--bcher-kva.example")
        );
        assert_eq!(
            normalize("xn--bcher-kva.example").as_deref(),
            Ok("xn--bcher-kva.example")
        );
        assert_eq!(normalize("例え.jp").as_deref(), Ok("xn--r8jz45g.jp"));
    }

    #[test]
    fn rejects_ip_literals_in_every_spelling() {
        for h in [
            "127.0.0.1",
            "8.8.8.8",
            "127.1",
            "2130706433",
            "0x7f.0.0.1",
            "0x7f000001",
            "[::1]",
            "::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "1.2.3.4.5",
        ] {
            assert!(
                matches!(normalize(h), Err(DenyReason::IpLiteral | DenyReason::NoDot)),
                "{h}: {:?}",
                normalize(h)
            );
        }
        assert_eq!(normalize("8.8.8.8"), Err(DenyReason::IpLiteral));
        assert_eq!(normalize("[::1]"), Err(DenyReason::IpLiteral));
    }

    #[test]
    fn rejects_userinfo_and_bad_syntax() {
        assert_eq!(normalize("user@example.com"), Err(DenyReason::Userinfo));
        assert_eq!(normalize("a b.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize("a/b.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize("a%2eb.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize("*.example.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize("exa_mple.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize("-bad.example.com"), Err(DenyReason::InvalidHost));
        assert_eq!(normalize(""), Err(DenyReason::InvalidHost));
        assert_eq!(
            normalize(&format!("{}.com", "a".repeat(64))),
            Err(DenyReason::InvalidHost)
        );
        assert_eq!(normalize("exa\u{0}mple.com"), Err(DenyReason::InvalidHost));
    }

    #[test]
    fn rejects_single_label_and_reserved_names() {
        assert_eq!(normalize("intranet"), Err(DenyReason::NoDot));
        assert_eq!(normalize("localhost"), Err(DenyReason::ReservedName));
        assert_eq!(normalize("LOCALHOST."), Err(DenyReason::ReservedName));
        for h in [
            "foo.localhost",
            "printer.local",
            "db.internal",
            "router.lan",
            "nas.home.arpa",
            "abc.onion",
            "1.0.0.127.in-addr.arpa",
            "x.ip6.arpa",
            "a.b.arpa",
        ] {
            assert_eq!(normalize(h), Err(DenyReason::ReservedName), "{h}");
        }
        // substring is not suffix
        assert!(normalize("notlocal.com").is_ok());
        assert!(normalize("internal.example.com").is_ok());
        assert!(normalize("lan.com").is_ok());
    }

    #[test]
    fn homographs_are_denied() {
        // Cyrillic a + Latin pple
        assert_eq!(normalize("аpple.com"), Err(DenyReason::MixedScript));
        // all-Cyrillic lookalike of "apple" (whole-script confusable)
        assert_eq!(normalize("аррӏе.com"), Err(DenyReason::MixedScript));
        // Greek omicron inside a Latin label
        assert_eq!(normalize("gοogle.com"), Err(DenyReason::MixedScript));
        // Latin + digits + Cyrillic
        assert_eq!(normalize("paypal1а.com"), Err(DenyReason::MixedScript));
        // Latin + Han
        assert_eq!(normalize("shop店.com"), Err(DenyReason::MixedScript));
        // unsupported script (Ethiopic)
        assert_eq!(normalize("አበባ.com"), Err(DenyReason::MixedScript));
        // the punycode form is judged the same way
        let p = idna::domain_to_ascii_strict("аpple.com").expect("valid idna");
        assert_eq!(normalize(&p), Err(DenyReason::MixedScript));
    }

    #[test]
    fn legitimate_non_latin_names_pass() {
        assert!(normalize("пример.рф").is_ok());
        assert!(normalize("яндекс.рф").is_ok());
        assert!(normalize("παράδειγμα.gr").is_ok());
        assert!(normalize("日本語.jp").is_ok());
        assert!(normalize("ひらがなカタカナ漢字.jp").is_ok());
        assert!(normalize("한국어.kr").is_ok());
        assert!(normalize("bücher.example").is_ok());
        assert!(normalize("münchen.de").is_ok());
        assert!(normalize("مثال.com").is_ok());
    }
}

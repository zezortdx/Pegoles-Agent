//! Allowlist entries: validated registrable domains or subdomains.

use super::host;
use super::DenyReason;

/// Most entries an allowlist may hold.
pub const MAX_ALLOWLIST: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AllowlistError {
    #[error("the allowlist is empty")]
    Empty,
    #[error("more than {MAX_ALLOWLIST} domains")]
    TooMany,
    #[error("wildcards are not allowed: a domain already covers its subdomains")]
    Wildcard,
    #[error("IP addresses are not allowed, only domain names")]
    IpLiteral,
    #[error("not a plain domain name (no scheme, path, port or credentials)")]
    InvalidSyntax,
    #[error("that is a public suffix such as \"com\" or \"co.uk\", not a registrable domain")]
    PublicSuffix,
    #[error("internal and local names cannot be allowed")]
    ReservedName,
}

/// A validated allowlist entry: lowercase ASCII (punycode), at least one
/// label below a public suffix, no IP literal, no wildcard.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Domain(String);

impl Domain {
    pub fn parse(input: &str) -> Result<Domain, AllowlistError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(AllowlistError::InvalidSyntax);
        }
        if input.contains('*') {
            return Err(AllowlistError::Wildcard);
        }
        if input.contains("://") || input.contains('/') || input.contains('@') {
            return Err(AllowlistError::InvalidSyntax);
        }
        let ascii = host::normalize(input).map_err(|r| match r {
            DenyReason::IpLiteral => AllowlistError::IpLiteral,
            DenyReason::ReservedName | DenyReason::NoDot => AllowlistError::ReservedName,
            _ => AllowlistError::InvalidSyntax,
        })?;
        // A public suffix (or a bare TLD) would allow every site under it.
        if psl::domain(ascii.as_bytes()).is_none() {
            return Err(AllowlistError::PublicSuffix);
        }
        Ok(Domain(ascii))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `host` (already normalized) equals this domain or is a subdomain.
    pub fn covers(&self, host: &str) -> bool {
        host == self.0
            || host
                .strip_suffix(self.0.as_str())
                .is_some_and(|rest| rest.ends_with('.'))
    }
}

/// Parses and validates a list (deduplicated, order kept).
pub fn parse_list<'a>(
    inputs: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<Domain>, AllowlistError> {
    let mut out: Vec<Domain> = Vec::new();
    for input in inputs {
        let d = Domain::parse(input)?;
        if !out.contains(&d) {
            out.push(d);
        }
        if out.len() > MAX_ALLOWLIST {
            return Err(AllowlistError::TooMany);
        }
    }
    if out.is_empty() {
        return Err(AllowlistError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_domains_and_subdomains() {
        assert_eq!(
            Domain::parse("Example.com").expect("ok").as_str(),
            "example.com"
        );
        assert_eq!(
            Domain::parse("docs.example.co.uk").expect("ok").as_str(),
            "docs.example.co.uk"
        );
        assert_eq!(
            Domain::parse("  wikipedia.org ").expect("ok").as_str(),
            "wikipedia.org"
        );
        assert_eq!(
            Domain::parse("bücher.de").expect("ok").as_str(),
            "xn--bcher-kva.de"
        );
    }

    #[test]
    fn rejects_wildcards_ips_and_syntax() {
        assert_eq!(
            Domain::parse("*.example.com"),
            Err(AllowlistError::Wildcard)
        );
        assert_eq!(Domain::parse("example.*"), Err(AllowlistError::Wildcard));
        assert_eq!(Domain::parse("8.8.8.8"), Err(AllowlistError::IpLiteral));
        assert_eq!(Domain::parse("[::1]"), Err(AllowlistError::IpLiteral));
        assert_eq!(
            Domain::parse("https://example.com"),
            Err(AllowlistError::InvalidSyntax)
        );
        assert_eq!(
            Domain::parse("example.com/path"),
            Err(AllowlistError::InvalidSyntax)
        );
        assert_eq!(
            Domain::parse("user@example.com"),
            Err(AllowlistError::InvalidSyntax)
        );
        assert_eq!(
            Domain::parse("example.com:443"),
            Err(AllowlistError::IpLiteral)
        );
        assert_eq!(Domain::parse(""), Err(AllowlistError::InvalidSyntax));
        assert_eq!(
            Domain::parse(".example.com"),
            Err(AllowlistError::InvalidSyntax)
        );
        assert_eq!(
            Domain::parse("exa mple.com"),
            Err(AllowlistError::InvalidSyntax)
        );
        assert_eq!(
            Domain::parse("аpple.com"),
            Err(AllowlistError::InvalidSyntax)
        );
    }

    #[test]
    fn rejects_public_suffixes_and_reserved_names() {
        for s in ["com", "co.uk", "github.io", "org"] {
            assert!(Domain::parse(s).is_err(), "{s}");
        }
        assert_eq!(Domain::parse("co.uk"), Err(AllowlistError::PublicSuffix));
        assert_eq!(
            Domain::parse("github.io"),
            Err(AllowlistError::PublicSuffix)
        );
        assert_eq!(
            Domain::parse("localhost"),
            Err(AllowlistError::ReservedName)
        );
        assert_eq!(
            Domain::parse("nas.local"),
            Err(AllowlistError::ReservedName)
        );
        assert!(Domain::parse("user.github.io").is_ok());
    }

    #[test]
    fn covers_is_equal_or_dot_boundary_subdomain() {
        let d = Domain::parse("example.com").expect("ok");
        assert!(d.covers("example.com"));
        assert!(d.covers("a.example.com"));
        assert!(d.covers("a.b.example.com"));
        assert!(!d.covers("badexample.com"));
        assert!(!d.covers("example.com.evil.com"));
        assert!(!d.covers("example.co"));
    }

    #[test]
    fn list_is_capped_deduplicated_and_non_empty() {
        let many: Vec<String> = (0..=MAX_ALLOWLIST)
            .map(|i| format!("site{i}.example.com"))
            .collect();
        assert_eq!(
            parse_list(many.iter().map(String::as_str)),
            Err(AllowlistError::TooMany)
        );
        let ok: Vec<String> = (0..MAX_ALLOWLIST)
            .map(|i| format!("site{i}.example.com"))
            .collect();
        assert_eq!(
            parse_list(ok.iter().map(String::as_str))
                .expect("32 is fine")
                .len(),
            32
        );
        assert_eq!(parse_list(["a.com", "A.COM"]).expect("dedup").len(), 1);
        assert_eq!(parse_list(Vec::<&str>::new()), Err(AllowlistError::Empty));
        assert_eq!(
            parse_list(["a.com", "*.b.com"]),
            Err(AllowlistError::Wildcard)
        );
    }
}

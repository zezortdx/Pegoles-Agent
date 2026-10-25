//! Transport rules (EGRESS.md section 1): what a proxy request may name.

use super::{host, DenyReason};

/// Methods accepted on absolute-form `http://` requests to the proxy.
const PLAIN_HTTP_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

/// A validated proxy target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub scheme: Scheme,
    /// Normalized ASCII host.
    pub host: String,
    pub port: u16,
    /// Path without query or fragment (`/` default). Empty for CONNECT: the
    /// path is only known per request after TLS interception.
    pub path: String,
    /// Origin-form request target (path and query, `/` default) for the
    /// upstream request line. Empty for CONNECT.
    pub origin: String,
}

/// Validates the method and request target of a request sent to the proxy.
pub fn parse_proxy_request(method: &str, target: &str) -> Result<Target, DenyReason> {
    if method == "CONNECT" {
        return parse_connect(target);
    }
    if !PLAIN_HTTP_METHODS.contains(&method) {
        return Err(DenyReason::MethodNotAllowed);
    }
    parse_absolute_http(target)
}

fn parse_connect(authority: &str) -> Result<Target, DenyReason> {
    if authority.contains('@') {
        return Err(DenyReason::Userinfo);
    }
    if authority.contains(['/', '?', '#']) {
        return Err(DenyReason::MalformedRequest);
    }
    let (host_part, port) = split_port(authority)?;
    let port = port.ok_or(DenyReason::PortNotAllowed)?;
    // Host rules first: an IP literal is refused as such, whatever its port.
    let host = host::normalize(host_part)?;
    if port != 443 {
        return Err(DenyReason::PortNotAllowed);
    }
    Ok(Target {
        scheme: Scheme::Https,
        host,
        port,
        path: String::new(),
        origin: String::new(),
    })
}

fn parse_absolute_http(target: &str) -> Result<Target, DenyReason> {
    let bytes = target.as_bytes();
    let rest = if bytes.len() >= 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
        &target[7..]
    } else if bytes.len() >= 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
        return Err(DenyReason::SchemeNotAllowed);
    } else {
        return Err(DenyReason::MalformedRequest);
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return Err(DenyReason::Userinfo);
    }
    let (host_part, port) = split_port(authority)?;
    let host = host::normalize(host_part)?;
    let port = port.unwrap_or(80);
    if port != 80 {
        return Err(DenyReason::PortNotAllowed);
    }
    Ok(Target {
        scheme: Scheme::Http,
        host,
        port,
        path: path_only(tail),
        origin: origin_form(tail),
    })
}

/// Splits `host[:port]`; bracketed IPv6 and bare colons are IP literals.
fn split_port(authority: &str) -> Result<(&str, Option<u16>), DenyReason> {
    if authority.starts_with('[') {
        return Err(DenyReason::IpLiteral);
    }
    match authority.rsplit_once(':') {
        None => Ok((authority, None)),
        Some((h, _)) if h.contains(':') => Err(DenyReason::IpLiteral),
        Some((h, p)) => {
            if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(DenyReason::MalformedRequest);
            }
            let port = p.parse::<u16>().map_err(|_| DenyReason::PortNotAllowed)?;
            Ok((h, Some(port)))
        }
    }
}

/// Request path for audit and URL matching: no query, no fragment, `/` if
/// empty. Not percent-decoded.
pub fn path_only(tail: &str) -> String {
    let end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..end];
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

/// Request target for the upstream: path and query, no fragment, `/` first.
fn origin_form(tail: &str) -> String {
    let end = tail.find('#').unwrap_or(tail.len());
    let t = &tail[..end];
    if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(method: &str, target: &str) -> Target {
        parse_proxy_request(method, target).unwrap_or_else(|e| panic!("{method} {target}: {e:?}"))
    }

    fn deny(method: &str, target: &str) -> DenyReason {
        parse_proxy_request(method, target).expect_err(target)
    }

    #[test]
    fn connect_443_and_absolute_http_are_the_only_forms() {
        let t = ok("CONNECT", "Example.com:443");
        assert_eq!(
            (t.scheme, t.host.as_str(), t.port, t.path.as_str()),
            (Scheme::Https, "example.com", 443, "")
        );
        let t = ok("GET", "http://Example.com/a/b?q=1#frag");
        assert_eq!(
            (t.scheme, t.host.as_str(), t.port, t.path.as_str()),
            (Scheme::Http, "example.com", 80, "/a/b")
        );
        assert_eq!(ok("POST", "http://example.com:80").path, "/");
        assert_eq!(ok("GET", "HTTP://example.com/x").host, "example.com");
        assert_eq!(ok("GET", "http://example.com?x=1").path, "/");
        assert_eq!(ok("GET", "http://example.com?x=1").origin, "/?x=1");
        assert_eq!(
            ok("GET", "http://example.com/a/b?q=1#frag").origin,
            "/a/b?q=1"
        );
        assert_eq!(ok("GET", "http://example.com").origin, "/");
    }

    #[test]
    fn other_methods_ports_and_schemes_are_denied() {
        assert_eq!(
            deny("TRACE", "http://example.com/"),
            DenyReason::MethodNotAllowed
        );
        assert_eq!(
            deny("get", "http://example.com/"),
            DenyReason::MethodNotAllowed
        );
        assert_eq!(
            deny("FOO", "http://example.com/"),
            DenyReason::MethodNotAllowed
        );
        assert_eq!(
            deny("CONNECT", "example.com:80"),
            DenyReason::PortNotAllowed
        );
        assert_eq!(
            deny("CONNECT", "example.com:8443"),
            DenyReason::PortNotAllowed
        );
        assert_eq!(deny("CONNECT", "example.com"), DenyReason::PortNotAllowed);
        assert_eq!(ok("CONNECT", "example.com:0443").port, 443);
        assert_eq!(
            deny("GET", "http://example.com:8080/"),
            DenyReason::PortNotAllowed
        );
        assert_eq!(
            deny("GET", "http://example.com:443/"),
            DenyReason::PortNotAllowed
        );
        assert_eq!(
            deny("GET", "https://example.com/"),
            DenyReason::SchemeNotAllowed
        );
        assert_eq!(
            deny("GET", "ftp://example.com/"),
            DenyReason::MalformedRequest
        );
        assert_eq!(deny("GET", "/relative"), DenyReason::MalformedRequest);
        assert_eq!(deny("GET", "*"), DenyReason::MalformedRequest);
        assert_eq!(
            deny("CONNECT", "example.com:99999"),
            DenyReason::PortNotAllowed
        );
        assert_eq!(
            deny("CONNECT", "example.com:"),
            DenyReason::MalformedRequest
        );
        assert_eq!(
            deny("CONNECT", "example.com:44a"),
            DenyReason::MalformedRequest
        );
    }

    #[test]
    fn userinfo_and_ip_literals_are_denied() {
        assert_eq!(
            deny("GET", "http://user:pw@example.com/"),
            DenyReason::Userinfo
        );
        assert_eq!(
            deny("GET", "http://example.com@evil.com/"),
            DenyReason::Userinfo
        );
        assert_eq!(
            deny("CONNECT", "user@example.com:443"),
            DenyReason::Userinfo
        );
        assert_eq!(deny("CONNECT", "8.8.8.8:443"), DenyReason::IpLiteral);
        assert_eq!(deny("CONNECT", "[2001:db8::1]:443"), DenyReason::IpLiteral);
        assert_eq!(deny("CONNECT", "[::1]:443"), DenyReason::IpLiteral);
        assert_eq!(deny("GET", "http://127.0.0.1/"), DenyReason::IpLiteral);
        assert_eq!(deny("GET", "http://[::1]/"), DenyReason::IpLiteral);
        assert_eq!(deny("GET", "http://2130706433/"), DenyReason::IpLiteral);
        assert_eq!(deny("GET", "http://0x7f.1/"), DenyReason::IpLiteral);
        assert_eq!(
            deny("CONNECT", "169.254.169.254:443"),
            DenyReason::IpLiteral
        );
    }

    #[test]
    fn reserved_and_dotless_hosts_are_denied() {
        assert_eq!(deny("CONNECT", "localhost:443"), DenyReason::ReservedName);
        assert_eq!(deny("GET", "http://localhost/"), DenyReason::ReservedName);
        assert_eq!(deny("GET", "http://intranet/"), DenyReason::NoDot);
        assert_eq!(
            deny("GET", "http://printer.local/"),
            DenyReason::ReservedName
        );
        assert_eq!(
            deny("CONNECT", "svc.internal:443"),
            DenyReason::ReservedName
        );
        assert_eq!(deny("CONNECT", "x.lan:443"), DenyReason::ReservedName);
        assert_eq!(
            deny("CONNECT", "router.home.arpa:443"),
            DenyReason::ReservedName
        );
        assert_eq!(
            deny("CONNECT", "abcdefg.onion:443"),
            DenyReason::ReservedName
        );
        assert_eq!(
            deny("CONNECT", "4.3.2.1.in-addr.arpa:443"),
            DenyReason::ReservedName
        );
    }

    #[test]
    fn homograph_host_is_denied() {
        assert_eq!(deny("CONNECT", "аpple.com:443"), DenyReason::MixedScript);
        assert_eq!(deny("GET", "http://аррӏе.com/"), DenyReason::MixedScript);
    }

    #[test]
    fn malformed_connect_targets_are_denied() {
        assert_eq!(
            deny("CONNECT", "example.com:443/path"),
            DenyReason::MalformedRequest
        );
        assert_eq!(
            deny("CONNECT", "http://example.com:443"),
            DenyReason::MalformedRequest
        );
    }
}

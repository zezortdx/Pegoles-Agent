//! The immutable protocol (EGRESS.md sections 1-4 and 6): closed enums and
//! exhaustive matches, no runtime configuration except the per-task mode and
//! allowlist.
//!
//! Decision order: transport rules, always-deny layer, mode rule (all pure,
//! no network), then resolution rules. The set of requests denied is the
//! order in the document; resolution runs last only because it needs the
//! network, and so that a host the policy already denies is never looked up
//! (a DNS query would itself be a covert channel).

mod allowlist;
mod host;
mod ip;
mod resolve;
mod transport;

use std::net::SocketAddr;
use std::sync::Arc;

use crate::threat::{Category, ThreatDb, ThreatKind};

pub use allowlist::{parse_list, AllowlistError, Domain, MAX_ALLOWLIST};
pub use host::normalize as normalize_host;
pub use ip::{blocked_range, is_global};
pub use resolve::{
    check_addresses, resolve_checked, ResolveFuture, Resolver, SystemResolver, RESOLVE_TIMEOUT,
};
pub use transport::{parse_proxy_request, path_only, Scheme, Target};

#[cfg(test)]
pub(crate) use resolve::testing;

/// Egress mode, chosen per task. Off by default, and `Off` never starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Allowlist(Vec<Domain>),
    OpenWeb,
}

/// Mode without its data, for audit events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeKind {
    Off,
    Allowlist,
    OpenWeb,
}

impl Mode {
    /// Validated allowlist mode (max 32 registrable domains or subdomains,
    /// no IPs, no wildcards).
    pub fn allowlist<'a>(
        domains: impl IntoIterator<Item = &'a str>,
    ) -> Result<Mode, AllowlistError> {
        parse_list(domains).map(Mode::Allowlist)
    }

    pub fn kind(&self) -> ModeKind {
        match self {
            Mode::Off => ModeKind::Off,
            Mode::Allowlist(_) => ModeKind::Allowlist,
            Mode::OpenWeb => ModeKind::OpenWeb,
        }
    }
}

impl ModeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModeKind::Off => "off",
            ModeKind::Allowlist => "allowlist",
            ModeKind::OpenWeb => "open_web",
        }
    }
}

/// Why something was refused or cut. Closed set; `code()` is the stable
/// reason code written to the audit log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DenyReason {
    // 1. transport rules
    MethodNotAllowed,
    PortNotAllowed,
    SchemeNotAllowed,
    Userinfo,
    IpLiteral,
    NoDot,
    ReservedName,
    InvalidHost,
    MixedScript,
    MalformedRequest,
    HeadTooLarge,
    HostMismatch,
    // 2. resolution rules
    ResolveFailed,
    NonGlobalAddress,
    // 3. always-deny layer
    ThreatMalware,
    ThreatPhishing,
    ThreatUrl,
    CategoryAdult,
    CategoryGambling,
    // 4. mode rule
    NotInAllowlist,
    // 5. response inspection
    BlockedSignature,
    DownloadNotSafe,
    DownloadTooLarge,
    EncodedBody,
    // 6. limits and upstream
    BodyTooLarge,
    SessionByteCap,
    TooManyStreams,
    UpstreamConnect,
    UpstreamTls,
    UpstreamProtocol,
    IdleTimeout,
}

impl DenyReason {
    pub fn code(self) -> &'static str {
        match self {
            DenyReason::MethodNotAllowed => "transport_method",
            DenyReason::PortNotAllowed => "transport_port",
            DenyReason::SchemeNotAllowed => "transport_scheme",
            DenyReason::Userinfo => "transport_userinfo",
            DenyReason::IpLiteral => "transport_ip_literal",
            DenyReason::NoDot => "transport_no_dot",
            DenyReason::ReservedName => "transport_reserved_name",
            DenyReason::InvalidHost => "transport_invalid_host",
            DenyReason::MixedScript => "transport_mixed_script",
            DenyReason::MalformedRequest => "transport_malformed",
            DenyReason::HeadTooLarge => "transport_head_too_large",
            DenyReason::HostMismatch => "transport_host_mismatch",
            DenyReason::ResolveFailed => "resolve_failed",
            DenyReason::NonGlobalAddress => "resolve_non_global",
            DenyReason::ThreatMalware => "threat_malware",
            DenyReason::ThreatPhishing => "threat_phishing",
            DenyReason::ThreatUrl => "threat_url",
            DenyReason::CategoryAdult => "category_adult",
            DenyReason::CategoryGambling => "category_gambling",
            DenyReason::NotInAllowlist => "mode_not_in_allowlist",
            DenyReason::BlockedSignature => "inspect_blocked_signature",
            DenyReason::DownloadNotSafe => "inspect_download_not_safe",
            DenyReason::DownloadTooLarge => "inspect_download_too_large",
            DenyReason::EncodedBody => "inspect_encoded_body",
            DenyReason::BodyTooLarge => "limit_body_too_large",
            DenyReason::SessionByteCap => "limit_session_bytes",
            DenyReason::TooManyStreams => "limit_streams",
            DenyReason::UpstreamConnect => "upstream_connect",
            DenyReason::UpstreamTls => "upstream_tls",
            DenyReason::UpstreamProtocol => "upstream_protocol",
            DenyReason::IdleTimeout => "limit_idle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(DenyReason),
}

impl Decision {
    pub fn reason_code(self) -> &'static str {
        match self {
            Decision::Allow => "allowed",
            Decision::Deny(r) => r.code(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("egress mode is off: the channel is never opened")]
    ModeOff,
    #[error("invalid allowlist: {0}")]
    Allowlist(#[from] AllowlistError),
    #[error("threat snapshot failed verification: {0}")]
    Threat(#[from] crate::threat::ThreatError),
}

/// A request that passed every rule: where to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorized {
    pub target: Target,
    /// Checked addresses; the proxy connects to these, never re-resolving.
    pub addrs: Vec<SocketAddr>,
}

/// The per-session policy: fixed mode plus the verified threat snapshot.
pub struct Policy {
    mode: Mode,
    threat: Arc<ThreatDb>,
}

impl Policy {
    /// Policy for `mode` with the compiled-in snapshot. Fails for
    /// `Mode::Off`, for an invalid allowlist, and if the snapshot does not
    /// match its SHA-256 constants (fail closed).
    pub fn new(mode: Mode) -> Result<Policy, PolicyError> {
        let threat = ThreatDb::builtin()?;
        Policy::with_threat(mode, threat)
    }

    pub(crate) fn with_threat(mode: Mode, threat: Arc<ThreatDb>) -> Result<Policy, PolicyError> {
        match &mode {
            Mode::Off => return Err(PolicyError::ModeOff),
            Mode::Allowlist(list) => {
                if list.is_empty() {
                    return Err(AllowlistError::Empty.into());
                }
                if list.len() > MAX_ALLOWLIST {
                    return Err(AllowlistError::TooMany.into());
                }
            }
            Mode::OpenWeb => {}
        }
        Ok(Policy { mode, threat })
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn mode_kind(&self) -> ModeKind {
        self.mode.kind()
    }

    /// Always-deny layer then mode rule for a normalized host. Pure.
    pub fn evaluate_host(&self, host: &str) -> Decision {
        if let Some(kind) = self.threat.host_threat(host) {
            return Decision::Deny(match kind {
                ThreatKind::Malware => DenyReason::ThreatMalware,
                ThreatKind::Phishing => DenyReason::ThreatPhishing,
            });
        }
        match &self.mode {
            Mode::Off => Decision::Deny(DenyReason::NotInAllowlist),
            Mode::Allowlist(list) => {
                if list.iter().any(|d| d.covers(host)) {
                    Decision::Allow
                } else {
                    Decision::Deny(DenyReason::NotInAllowlist)
                }
            }
            Mode::OpenWeb => match self.threat.category(host) {
                Some(Category::Adult) => Decision::Deny(DenyReason::CategoryAdult),
                Some(Category::Gambling) => Decision::Deny(DenyReason::CategoryGambling),
                None => Decision::Allow,
            },
        }
    }

    /// URL-level part of the always-deny layer (threat URLs), for one
    /// request. Applies in both modes and beats the allowlist.
    pub fn evaluate_url(&self, host: &str, path: &str) -> Decision {
        if self.threat.url_threat(host, path) {
            Decision::Deny(DenyReason::ThreatUrl)
        } else {
            Decision::Allow
        }
    }

    /// The whole pipeline for a request sent to the proxy: transport rules,
    /// always-deny layer, mode rule, then resolution. Returns the checked
    /// addresses to connect to.
    pub async fn authorize(
        &self,
        resolver: &dyn Resolver,
        method: &str,
        target: &str,
    ) -> Result<Authorized, DenyReason> {
        let target = parse_proxy_request(method, target)?;
        self.authorize_target(resolver, target).await
    }

    /// Like [`Policy::authorize`] for an already parsed target.
    pub async fn authorize_target(
        &self,
        resolver: &dyn Resolver,
        target: Target,
    ) -> Result<Authorized, DenyReason> {
        if let Decision::Deny(r) = self.evaluate_host(&target.host) {
            return Err(r);
        }
        if target.scheme == Scheme::Http {
            if let Decision::Deny(r) = self.evaluate_url(&target.host, &target.path) {
                return Err(r);
            }
        }
        let addrs = resolve_checked(resolver, &target.host, target.port).await?;
        Ok(Authorized { target, addrs })
    }
}

#[cfg(test)]
mod tests;

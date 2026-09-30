use std::sync::Arc;

use super::testing::TableResolver;
use super::*;
use crate::threat::ThreatDb;

fn db() -> Arc<ThreatDb> {
    ThreatDb::from_text(
        "malware.example\n",
        "phish.example\n",
        "host.example/bad/payload\n",
        "adult.example\n",
        "casino.example\n",
    )
}

fn open() -> Policy {
    Policy::with_threat(Mode::OpenWeb, db()).expect("policy")
}

fn allow(domains: &[&str]) -> Policy {
    let mode = Mode::allowlist(domains.iter().copied()).expect("allowlist");
    Policy::with_threat(mode, db()).expect("policy")
}

fn resolver() -> TableResolver {
    TableResolver::with(&[
        ("ok.example.com", &["93.184.216.34"]),
        ("docs.ok.example.com", &["93.184.216.35"]),
        ("malware.example", &["93.184.216.36"]),
        ("phish.example", &["93.184.216.37"]),
        ("adult.example", &["93.184.216.38"]),
        ("casino.example", &["93.184.216.39"]),
        ("host.example", &["93.184.216.40"]),
        ("rebind.example.com", &["127.0.0.1"]),
        ("meta.example.com", &["169.254.169.254"]),
        ("other.example.org", &["93.184.216.41"]),
    ])
}

async fn run(p: &Policy, method: &str, target: &str) -> Result<Authorized, DenyReason> {
    p.authorize(&resolver(), method, target).await
}

#[test]
fn off_refuses_to_start() {
    assert_eq!(
        Policy::with_threat(Mode::Off, db()).err(),
        Some(PolicyError::ModeOff)
    );
    assert_eq!(Policy::new(Mode::Off).err(), Some(PolicyError::ModeOff));
}

#[test]
fn hand_built_oversized_or_empty_allowlists_are_rejected_at_start() {
    let d = Domain::parse("a.example.com").expect("domain");
    assert_eq!(
        Policy::with_threat(Mode::Allowlist(vec![d.clone(); 33]), db()).err(),
        Some(PolicyError::Allowlist(AllowlistError::TooMany))
    );
    assert_eq!(
        Policy::with_threat(Mode::Allowlist(vec![]), db()).err(),
        Some(PolicyError::Allowlist(AllowlistError::Empty))
    );
}

#[test]
fn builtin_policy_starts_in_both_active_modes() {
    assert!(Policy::new(Mode::OpenWeb).is_ok());
    assert!(Policy::new(Mode::allowlist(["example.com"]).expect("list")).is_ok());
}

#[tokio::test]
async fn open_web_allows_public_hosts_and_returns_checked_addresses() {
    let ok = run(&open(), "CONNECT", "ok.example.com:443")
        .await
        .expect("allowed");
    assert_eq!(ok.target.host, "ok.example.com");
    assert_eq!(ok.addrs, vec!["93.184.216.34:443".parse().expect("addr")]);
    let ok = run(&open(), "GET", "http://other.example.org/a?b=1")
        .await
        .expect("allowed");
    assert_eq!(ok.addrs[0].port(), 80);
}

#[tokio::test]
async fn allowlist_covers_the_domain_and_its_subdomains_only() {
    let p = allow(&["ok.example.com"]);
    assert!(run(&p, "CONNECT", "ok.example.com:443").await.is_ok());
    assert!(run(&p, "CONNECT", "docs.ok.example.com:443").await.is_ok());
    assert_eq!(
        run(&p, "CONNECT", "other.example.org:443").await,
        Err(DenyReason::NotInAllowlist)
    );
    assert_eq!(
        run(&p, "CONNECT", "evilok.example.com:443").await,
        Err(DenyReason::NotInAllowlist)
    );
}

#[tokio::test]
async fn always_deny_beats_the_allowlist_and_needs_no_resolution() {
    // The threat host is explicitly allowlisted and resolves fine.
    let p = allow(&["malware.example", "phish.example", "host.example"]);
    assert_eq!(
        run(&p, "CONNECT", "malware.example:443").await,
        Err(DenyReason::ThreatMalware)
    );
    assert_eq!(
        run(&p, "CONNECT", "sub.malware.example:443").await,
        Err(DenyReason::ThreatMalware)
    );
    assert_eq!(
        run(&p, "CONNECT", "phish.example:443").await,
        Err(DenyReason::ThreatPhishing)
    );
    assert_eq!(
        run(&p, "GET", "http://host.example/bad/payload?x=1").await,
        Err(DenyReason::ThreatUrl)
    );
    assert!(run(&p, "GET", "http://host.example/good").await.is_ok());
    // also in open web
    assert_eq!(
        run(&open(), "CONNECT", "malware.example:443").await,
        Err(DenyReason::ThreatMalware)
    );
}

#[tokio::test]
async fn categories_apply_in_open_web_only() {
    assert_eq!(
        run(&open(), "CONNECT", "adult.example:443").await,
        Err(DenyReason::CategoryAdult)
    );
    assert_eq!(
        run(&open(), "CONNECT", "x.casino.example:443").await,
        Err(DenyReason::CategoryGambling)
    );
    let p = allow(&["adult.example", "casino.example"]);
    assert!(run(&p, "CONNECT", "adult.example:443").await.is_ok());
    assert!(run(&p, "CONNECT", "casino.example:443").await.is_ok());
}

#[tokio::test]
async fn dns_rebinding_to_a_private_address_is_denied_in_both_modes() {
    let p = allow(&["example.com"]);
    assert_eq!(
        run(&p, "CONNECT", "rebind.example.com:443").await,
        Err(DenyReason::NonGlobalAddress)
    );
    assert_eq!(
        run(&open(), "CONNECT", "meta.example.com:443").await,
        Err(DenyReason::NonGlobalAddress)
    );
    assert_eq!(
        run(&open(), "GET", "http://rebind.example.com/").await,
        Err(DenyReason::NonGlobalAddress)
    );
}

#[tokio::test]
async fn unresolvable_host_is_resolve_failed() {
    assert_eq!(
        run(&open(), "CONNECT", "nx.example.net:443").await,
        Err(DenyReason::ResolveFailed)
    );
}

#[tokio::test]
async fn hosts_the_policy_denies_are_never_resolved() {
    struct Panicky;
    impl Resolver for Panicky {
        fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
            panic!("resolved {host}: a denied host must not be looked up");
        }
    }
    let p = allow(&["ok.example.com"]);
    assert_eq!(
        p.authorize(&Panicky, "CONNECT", "other.example.org:443")
            .await,
        Err(DenyReason::NotInAllowlist)
    );
    assert_eq!(
        open()
            .authorize(&Panicky, "CONNECT", "malware.example:443")
            .await,
        Err(DenyReason::ThreatMalware)
    );
    assert_eq!(
        open().authorize(&Panicky, "CONNECT", "127.0.0.1:443").await,
        Err(DenyReason::IpLiteral)
    );
}

#[tokio::test]
async fn transport_rules_apply_before_anything_else() {
    let p = allow(&["ok.example.com"]);
    assert_eq!(
        run(&p, "CONNECT", "ok.example.com:80").await,
        Err(DenyReason::PortNotAllowed)
    );
    assert_eq!(
        run(&p, "TRACE", "http://ok.example.com/").await,
        Err(DenyReason::MethodNotAllowed)
    );
    assert_eq!(
        run(&p, "GET", "http://u:p@ok.example.com/").await,
        Err(DenyReason::Userinfo)
    );
    assert_eq!(
        run(&p, "CONNECT", "localhost:443").await,
        Err(DenyReason::ReservedName)
    );
    assert_eq!(
        run(&p, "CONNECT", "аpple.com:443").await,
        Err(DenyReason::MixedScript)
    );
}

#[tokio::test]
async fn idn_allowlist_entry_matches_its_punycode_host() {
    let p = allow(&["bücher.de"]);
    let r = TableResolver::with(&[("xn--bcher-kva.de", &["93.184.216.50"])]);
    assert!(p.authorize(&r, "CONNECT", "BÜCHER.de:443").await.is_ok());
    assert!(p
        .authorize(&r, "CONNECT", "xn--bcher-kva.de:443")
        .await
        .is_ok());
}

#[test]
fn every_deny_reason_has_a_distinct_nonempty_code() {
    let all = [
        DenyReason::MethodNotAllowed,
        DenyReason::PortNotAllowed,
        DenyReason::SchemeNotAllowed,
        DenyReason::Userinfo,
        DenyReason::IpLiteral,
        DenyReason::NoDot,
        DenyReason::ReservedName,
        DenyReason::InvalidHost,
        DenyReason::MixedScript,
        DenyReason::MalformedRequest,
        DenyReason::HeadTooLarge,
        DenyReason::HostMismatch,
        DenyReason::ResolveFailed,
        DenyReason::NonGlobalAddress,
        DenyReason::ThreatMalware,
        DenyReason::ThreatPhishing,
        DenyReason::ThreatUrl,
        DenyReason::CategoryAdult,
        DenyReason::CategoryGambling,
        DenyReason::NotInAllowlist,
        DenyReason::BlockedSignature,
        DenyReason::DownloadNotSafe,
        DenyReason::DownloadTooLarge,
        DenyReason::EncodedBody,
        DenyReason::BodyTooLarge,
        DenyReason::SessionByteCap,
        DenyReason::TooManyStreams,
        DenyReason::UpstreamConnect,
        DenyReason::UpstreamTls,
        DenyReason::IdleTimeout,
    ];
    let mut codes: Vec<&str> = all.iter().map(|r| r.code()).collect();
    assert!(codes.iter().all(|c| !c.is_empty()));
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
    assert_eq!(Decision::Allow.reason_code(), "allowed");
}

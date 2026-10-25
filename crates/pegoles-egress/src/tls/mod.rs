//! TLS interception (EGRESS.md, "TLS interception").
//!
//! - A fresh CA (ECDSA P-256, 24 h) per egress session, generated in memory;
//!   its key never touches disk and dies with the session.
//! - Leaf certificates are minted per host and cached per session. Toward
//!   the guest only `http/1.1` is offered.
//! - Upstream, certificates are verified against `webpki-roots` only: no
//!   system trust store, no user overrides, no click-through.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use time::{Duration, OffsetDateTime};

use pegoles_egress_proto::MAX_CA_DER;

/// CA and leaf lifetime.
const CA_VALIDITY: Duration = Duration::hours(24);
/// Backdating for guests whose clock lags the host.
const CLOCK_SKEW: Duration = Duration::minutes(10);
/// Leaf configs kept per session before the cache is reset.
const MAX_CACHED_LEAVES: usize = 512;

const ALPN_HTTP11: &[u8] = b"http/1.1";

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("certificate generation failed: {0}")]
    Generate(String),
    #[error("TLS configuration failed: {0}")]
    Config(String),
    #[error("the session CA certificate does not fit the HELLO frame")]
    CaTooLarge,
    #[error("invalid host name for a certificate")]
    BadHost,
}

fn gen_err(e: rcgen::Error) -> TlsError {
    TlsError::Generate(e.to_string())
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The per-session CA and its leaf cache.
pub struct SessionCa {
    params: CertificateParams,
    key: KeyPair,
    der: Vec<u8>,
    not_after: OffsetDateTime,
    leaves: Mutex<HashMap<String, Arc<ServerConfig>>>,
}

impl SessionCa {
    pub fn generate() -> Result<SessionCa, TlsError> {
        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(gen_err)?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Pegoles egress session CA");
        dn.push(DnType::OrganizationName, "Pegoles");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.not_before = now - CLOCK_SKEW;
        params.not_after = now + CA_VALIDITY;
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(gen_err)?;
        let cert = params.self_signed(&key).map_err(gen_err)?;
        let der = cert.der().to_vec();
        if der.is_empty() || der.len() > MAX_CA_DER {
            return Err(TlsError::CaTooLarge);
        }
        let not_after = params.not_after;
        Ok(SessionCa {
            params,
            key,
            der,
            not_after,
            leaves: Mutex::new(HashMap::new()),
        })
    }

    /// CA certificate (DER) for the HELLO frame.
    pub fn ca_der(&self) -> &[u8] {
        &self.der
    }

    /// TLS server configuration presenting a leaf for `host` (normalized
    /// ASCII), cached per session.
    pub fn server_config(&self, host: &str) -> Result<Arc<ServerConfig>, TlsError> {
        if let Some(cfg) = self.cached(host) {
            return Ok(cfg);
        }
        let cfg = Arc::new(self.mint(host)?);
        let mut cache = self
            .leaves
            .lock()
            .map_err(|_| TlsError::Config("leaf cache poisoned".into()))?;
        if cache.len() >= MAX_CACHED_LEAVES {
            cache.clear();
        }
        cache.insert(host.to_string(), cfg.clone());
        Ok(cfg)
    }

    fn cached(&self, host: &str) -> Option<Arc<ServerConfig>> {
        self.leaves.lock().ok()?.get(host).cloned()
    }

    fn mint(&self, host: &str) -> Result<ServerConfig, TlsError> {
        if host.is_empty() || host.len() > 253 || !host.is_ascii() {
            return Err(TlsError::BadHost);
        }
        let now = OffsetDateTime::now_utc();
        let mut params =
            CertificateParams::new(vec![host.to_string()]).map_err(|_| TlsError::BadHost)?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host);
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = now - CLOCK_SKEW;
        params.not_after = self.not_after - Duration::minutes(1);
        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(gen_err)?;
        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).map_err(gen_err)?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let mut cfg = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Config(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(cert.der().to_vec())], key)
            .map_err(|e| TlsError::Config(e.to_string()))?;
        cfg.alpn_protocols = vec![ALPN_HTTP11.to_vec()];
        Ok(cfg)
    }
}

/// Client configuration for the upstream side: `webpki-roots` only, TLS 1.2+,
/// `http/1.1`.
pub fn upstream_config() -> Result<Arc<ClientConfig>, TlsError> {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    client_config(roots)
}

/// Like [`upstream_config`] plus one extra trusted root. Exists only in test
/// builds: release builds cannot add roots.
#[cfg(test)]
pub(crate) fn upstream_config_with_root(root_der: &[u8]) -> Result<Arc<ClientConfig>, TlsError> {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    roots
        .add(CertificateDer::from(root_der.to_vec()))
        .map_err(|e| TlsError::Config(e.to_string()))?;
    client_config(roots)
}

fn client_config(roots: RootCertStore) -> Result<Arc<ClientConfig>, TlsError> {
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![ALPN_HTTP11.to_vec()];
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_fits_the_hello_frame_and_leaves_are_cached() {
        let ca = SessionCa::generate().expect("ca");
        assert!(!ca.ca_der().is_empty() && ca.ca_der().len() <= MAX_CA_DER);
        let a = ca.server_config("example.com").expect("leaf");
        let b = ca.server_config("example.com").expect("leaf");
        assert!(Arc::ptr_eq(&a, &b), "second call hits the cache");
        let c = ca.server_config("other.example").expect("leaf");
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(a.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn every_session_gets_a_different_ca() {
        let a = SessionCa::generate().expect("ca");
        let b = SessionCa::generate().expect("ca");
        assert_ne!(a.ca_der(), b.ca_der());
    }

    #[test]
    fn bad_hosts_are_refused() {
        let ca = SessionCa::generate().expect("ca");
        assert!(ca.server_config("").is_err());
        assert!(ca.server_config("bücher.example").is_err());
        assert!(ca.server_config(&"a".repeat(300)).is_err());
    }

    #[test]
    fn cache_is_bounded() {
        let ca = SessionCa::generate().expect("ca");
        for i in 0..(MAX_CACHED_LEAVES + 3) {
            ca.server_config(&format!("h{i}.example")).expect("leaf");
        }
        let n = ca.leaves.lock().map(|c| c.len()).unwrap_or(usize::MAX);
        assert!(n <= MAX_CACHED_LEAVES);
    }

    #[test]
    fn upstream_config_offers_http11_only() {
        let cfg = upstream_config().expect("config");
        assert_eq!(cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }
}

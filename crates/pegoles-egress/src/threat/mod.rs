//! Compiled-in threat snapshot (EGRESS.md section 3).
//!
//! The data files in `crates/pegoles-egress/data` are embedded with
//! `include_bytes!`, each checked against a SHA-256 constant
//! (`digests.rs`, written by `scripts/egress/update-threat-feed.sh`) before
//! it is parsed. A mismatch is an error and egress refuses to start.
//!
//! Lookups hash the normalized name with SHA-256 (first 8 bytes) into a
//! sorted `Vec<u64>` and binary-search it: O(log n), ~8 bytes per entry.
//! Domain lists match the host and every parent domain.

mod digests;
mod urlnorm;

use std::io::Read;
use std::sync::{Arc, OnceLock};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

/// Decompression bound per list (the files are far smaller).
const MAX_DECOMPRESSED: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatKind {
    Malware,
    Phishing,
}

/// Content categories, blocked in open web mode only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Adult,
    Gambling,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThreatError {
    #[error("threat list {list} does not match its compiled-in SHA-256")]
    DigestMismatch { list: &'static str },
    #[error("threat list {list} is not valid gzip or exceeds its size bound")]
    Corrupt { list: &'static str },
    #[error("threat list {list} is empty")]
    Empty { list: &'static str },
}

/// Sorted 64-bit name hashes.
#[derive(Default)]
struct HashSet64(Vec<u64>);

impl HashSet64 {
    #[cfg(test)]
    fn from_text(text: &str) -> HashSet64 {
        HashSet64::from_text_with(text, |l| l.to_string())
    }

    /// Like [`from_text`], hashing `norm(line)`.
    fn from_text_with(text: &str, norm: impl Fn(&str) -> String) -> HashSet64 {
        let mut v: Vec<u64> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| hash_key(&norm(l)))
            .collect();
        v.sort_unstable();
        v.dedup();
        HashSet64(v)
    }

    fn contains(&self, key: &str) -> bool {
        self.0.binary_search(&hash_key(key)).is_ok()
    }

    /// `host` or any parent domain with at least two labels.
    fn contains_domain(&self, host: &str) -> bool {
        let mut h = host;
        loop {
            if self.contains(h) {
                return true;
            }
            match h.split_once('.') {
                Some((_, rest)) if rest.contains('.') => h = rest,
                _ => return false,
            }
        }
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

fn hash_key(key: &str) -> u64 {
    let d = Sha256::digest(key.as_bytes());
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    u64::from_le_bytes(b)
}

/// The verified snapshot.
pub struct ThreatDb {
    malware: HashSet64,
    phishing: HashSet64,
    urls: HashSet64,
    adult: HashSet64,
    gambling: HashSet64,
}

/// Checks `bytes` against `expected` (lowercase hex SHA-256).
fn verify(list: &'static str, bytes: &[u8], expected: &str) -> Result<(), ThreatError> {
    if hex::encode(Sha256::digest(bytes)) == expected {
        Ok(())
    } else {
        Err(ThreatError::DigestMismatch { list })
    }
}

fn gunzip(list: &'static str, bytes: &[u8]) -> Result<String, ThreatError> {
    let mut out = String::new();
    let mut rd = GzDecoder::new(bytes).take(MAX_DECOMPRESSED + 1);
    rd.read_to_string(&mut out)
        .map_err(|_| ThreatError::Corrupt { list })?;
    if out.len() as u64 > MAX_DECOMPRESSED {
        return Err(ThreatError::Corrupt { list });
    }
    Ok(out)
}

fn load(list: &'static str, bytes: &[u8], expected: &str) -> Result<HashSet64, ThreatError> {
    load_with(list, bytes, expected, |l| l.to_string())
}

fn load_with(
    list: &'static str,
    bytes: &[u8],
    expected: &str,
    norm: impl Fn(&str) -> String,
) -> Result<HashSet64, ThreatError> {
    verify(list, bytes, expected)?;
    let set = HashSet64::from_text_with(&gunzip(list, bytes)?, norm);
    if set.len() == 0 {
        return Err(ThreatError::Empty { list });
    }
    Ok(set)
}

impl ThreatDb {
    /// The compiled-in snapshot, verified once per process. Fails closed.
    pub fn builtin() -> Result<Arc<ThreatDb>, ThreatError> {
        static CELL: OnceLock<Result<Arc<ThreatDb>, ThreatError>> = OnceLock::new();
        CELL.get_or_init(|| ThreatDb::load_builtin().map(Arc::new))
            .clone()
    }

    fn load_builtin() -> Result<ThreatDb, ThreatError> {
        Ok(ThreatDb {
            malware: load(
                "malware-domains",
                include_bytes!("../../data/malware-domains.txt.gz"),
                digests::MALWARE_DOMAINS_SHA256,
            )?,
            phishing: load(
                "phishing-domains",
                include_bytes!("../../data/phishing-domains.txt.gz"),
                digests::PHISHING_DOMAINS_SHA256,
            )?,
            // Normalized at load time, so the vendored data and its digest
            // stay as downloaded.
            urls: load_with(
                "malware-urls",
                include_bytes!("../../data/malware-urls.txt.gz"),
                digests::MALWARE_URLS_SHA256,
                urlnorm::normalize_entry,
            )?,
            adult: load(
                "adult-domains",
                include_bytes!("../../data/adult-domains.txt.gz"),
                digests::ADULT_DOMAINS_SHA256,
            )?,
            gambling: load(
                "gambling-domains",
                include_bytes!("../../data/gambling-domains.txt.gz"),
                digests::GAMBLING_DOMAINS_SHA256,
            )?,
        })
    }

    /// Malware or phishing domain (host or any parent domain listed).
    pub fn host_threat(&self, host: &str) -> Option<ThreatKind> {
        if self.malware.contains_domain(host) {
            Some(ThreatKind::Malware)
        } else if self.phishing.contains_domain(host) {
            Some(ThreatKind::Phishing)
        } else {
            None
        }
    }

    /// Match of the normalized `host/path` (see `urlnorm`) against the
    /// malware URL list (query and fragment already removed).
    pub fn url_threat(&self, host: &str, path: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        let key = urlnorm::key(host, path);
        self.urls.contains(&key)
    }

    /// Adult / gambling category (host or any parent domain listed).
    pub fn category(&self, host: &str) -> Option<Category> {
        if self.adult.contains_domain(host) {
            Some(Category::Adult)
        } else if self.gambling.contains_domain(host) {
            Some(Category::Gambling)
        } else {
            None
        }
    }

    /// Entry counts: malware, phishing, urls, adult, gambling.
    pub fn sizes(&self) -> [usize; 5] {
        [
            self.malware.len(),
            self.phishing.len(),
            self.urls.len(),
            self.adult.len(),
            self.gambling.len(),
        ]
    }

    /// Snapshot from plain text lists (one entry per line); tests only.
    #[cfg(test)]
    pub(crate) fn from_text(
        malware: &str,
        phishing: &str,
        urls: &str,
        adult: &str,
        gambling: &str,
    ) -> Arc<ThreatDb> {
        Arc::new(ThreatDb {
            malware: HashSet64::from_text(malware),
            phishing: HashSet64::from_text(phishing),
            urls: HashSet64::from_text_with(urls, urlnorm::normalize_entry),
            adult: HashSet64::from_text(adult),
            gambling: HashSet64::from_text(gambling),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_line(bytes: &[u8]) -> String {
        gunzip("test", bytes)
            .expect("gunzip")
            .lines()
            .find(|l| !l.is_empty())
            .expect("non-empty list")
            .to_string()
    }

    #[test]
    fn builtin_snapshot_verifies_and_is_populated() {
        let db = ThreatDb::builtin().expect("snapshot must verify");
        let [m, p, u, a, g] = db.sizes();
        assert!(m > 500, "malware {m}");
        assert!(p > 100_000, "phishing {p}");
        assert!(u > 5_000, "urls {u}");
        assert!(a > 10_000, "adult {a}");
        assert!(g > 1_000, "gambling {g}");
    }

    #[test]
    fn digest_mismatch_fails_closed() {
        let good = include_bytes!("../../data/malware-domains.txt.gz");
        assert!(load("m", good, digests::MALWARE_DOMAINS_SHA256).is_ok());
        let mut bad = good.to_vec();
        bad[20] ^= 1;
        assert_eq!(
            load("m", &bad, digests::MALWARE_DOMAINS_SHA256).err(),
            Some(ThreatError::DigestMismatch { list: "m" })
        );
        assert_eq!(
            load("m", good, &"0".repeat(64)).err(),
            Some(ThreatError::DigestMismatch { list: "m" })
        );
    }

    #[test]
    fn garbage_with_a_matching_digest_is_corrupt_not_a_panic() {
        let junk = b"not gzip at all";
        let digest = hex::encode(Sha256::digest(junk));
        assert_eq!(
            load("junk", junk, &digest).err(),
            Some(ThreatError::Corrupt { list: "junk" })
        );
    }

    #[test]
    fn first_entry_of_every_list_is_found_with_its_subdomains() {
        let db = ThreatDb::builtin().expect("snapshot");
        let m = first_line(include_bytes!("../../data/malware-domains.txt.gz"));
        assert_eq!(db.host_threat(&m), Some(ThreatKind::Malware), "{m}");
        assert_eq!(
            db.host_threat(&format!("sub.deep.{m}")),
            Some(ThreatKind::Malware)
        );
        let p = first_line(include_bytes!("../../data/phishing-domains.txt.gz"));
        assert!(db.host_threat(&p).is_some(), "{p}");
        let a = first_line(include_bytes!("../../data/adult-domains.txt.gz"));
        assert_eq!(db.category(&a), Some(Category::Adult), "{a}");
        let g = first_line(include_bytes!("../../data/gambling-domains.txt.gz"));
        assert!(db.category(&g).is_some(), "{g}");
        let u = first_line(include_bytes!("../../data/malware-urls.txt.gz"));
        let (host, path) = u.split_once('/').expect("host/path");
        assert!(db.url_threat(host, &format!("/{path}")), "{u}");
    }

    #[test]
    fn well_known_sites_are_not_listed() {
        let db = ThreatDb::builtin().expect("snapshot");
        for h in [
            "example.com",
            "wikipedia.org",
            "github.com",
            "www.google.com",
            "docs.rs",
        ] {
            assert_eq!(db.host_threat(h), None, "{h}");
            assert_eq!(db.category(h), None, "{h}");
        }
    }

    #[test]
    fn parent_domains_match_but_siblings_and_lookalikes_do_not() {
        let db = ThreatDb::from_text(
            "evil.example\nx.co.uk",
            "",
            "bad.test/dl/a.exe",
            "adult.test",
            "",
        );
        assert_eq!(db.host_threat("evil.example"), Some(ThreatKind::Malware));
        assert_eq!(
            db.host_threat("a.b.evil.example"),
            Some(ThreatKind::Malware)
        );
        assert_eq!(db.host_threat("notevil.example"), None);
        assert_eq!(db.host_threat("evil.example.org"), None);
        assert_eq!(db.host_threat("example"), None);
        assert_eq!(db.host_threat("a.x.co.uk"), Some(ThreatKind::Malware));
        assert_eq!(db.host_threat("co.uk"), None);
        assert_eq!(db.category("www.adult.test"), Some(Category::Adult));
        assert!(db.url_threat("bad.test", "/dl/a.exe"));
        assert!(!db.url_threat("bad.test", "/dl/b.exe"));
        assert!(!db.url_threat("www.bad.test", "/dl/a.exe"));
        assert!(!db.url_threat("bad.test", ""));
    }

    #[test]
    fn url_lookups_see_through_path_obfuscation() {
        let db = ThreatDb::from_text("", "", "bad.test/dl/a.exe\nbad.test/q/w.exe\n", "", "");
        for path in [
            "/dl/a.exe",
            "/dl//a.exe",
            "//dl/a.exe",
            "/dl/./a.exe",
            "/x/../dl/a.exe",
            "/dl/x/../../dl/a.exe",
            "/dl/%61.exe",
            "/dl/a%2eexe",
            "/dl/a%2Eexe",
            "/%64l/a.exe",
            "/dl/a.exe;jsessionid=1",
            "/dl;x=1/a.exe",
            "/dl/a.exe/",
            "/dl/a.exe/.",
            "/dl/b.exe/../a.exe",
            "/../../dl/a.exe",
        ] {
            let want = !path.ends_with('/') && !path.ends_with("/.");
            assert_eq!(db.url_threat("bad.test", path), want, "{path}");
        }
        assert!(db.url_threat("BAD.Test", "/dl/a.exe"), "host case");
        assert!(db.url_threat("bad.test.", "/dl/a.exe"), "trailing dot");
        // a reserved escape is data, not a separator
        assert!(!db.url_threat("bad.test", "/dl%2Fa.exe"));
        assert!(!db.url_threat("bad.test", "/dl%2fa.exe"));
        assert!(!db.url_threat("bad.test", "/dl/a.exe%00"));
        assert!(!db.url_threat("bad.test", "/dl/b.exe"));
        assert!(
            !db.url_threat("bad.test", "/dl/A.exe"),
            "paths stay case-sensitive"
        );
        assert!(!db.url_threat("www.bad.test", "/dl/a.exe"));
    }

    #[test]
    fn list_entries_are_normalized_when_loaded() {
        let db = ThreatDb::from_text(
            "",
            "",
            "Bad.Test//dl/./a.exe\nbad.test/x/%61/../b.exe;v=1\n",
            "",
            "",
        );
        assert!(db.url_threat("bad.test", "/dl/a.exe"));
        assert!(db.url_threat("bad.test", "/x/b.exe"));
        assert_eq!(db.sizes()[2], 2);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let db = ThreatDb::from_text("# c\n\n  spaced.example  \n", "", "", "", "");
        assert_eq!(db.host_threat("spaced.example"), Some(ThreatKind::Malware));
        assert_eq!(db.sizes()[0], 1);
    }
}

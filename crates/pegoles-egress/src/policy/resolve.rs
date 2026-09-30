//! Resolution rules (EGRESS.md section 2): the host resolves names itself,
//! every address must be global unicast, and the proxy connects to the
//! checked address (no second lookup, so no DNS rebinding).

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use tokio::sync::Semaphore;

use super::ip::blocked_range;
use super::DenyReason;

/// Longest a name lookup may take.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
/// Most name lookups in flight at once. A `getaddrinfo` call on the
/// blocking pool cannot be cancelled, so the cap counts threads actually
/// busy in the OS, not futures: a saturated resolver denies (`ResolveFailed`)
/// instead of queueing.
pub const MAX_LOOKUPS: usize = 16;
static LOOKUP_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_LOOKUPS)));
/// Most addresses kept from one answer.
const MAX_ADDRS: usize = 8;

/// Boxed future returned by [`Resolver::resolve`].
pub type ResolveFuture<'a> = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>>;

/// Name resolution, injectable so tests control the answers.
pub trait Resolver: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a>;
}

/// The operating system resolver (`getaddrinfo` on the blocking pool).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        let host = host.to_string();
        Box::pin(async move {
            bounded_lookup(LOOKUP_SLOTS.clone(), move || {
                Ok((host.as_str(), 0)
                    .to_socket_addrs()?
                    .map(|a| a.ip())
                    .collect())
            })
            .await
        })
    }
}

/// Runs `lookup` on the blocking pool under one of `slots`' permits. The
/// permit is released when the lookup itself ends, not when the caller
/// gives up (timeout, aborted stream), so abandoned lookups still count.
pub(crate) async fn bounded_lookup<F>(slots: Arc<Semaphore>, lookup: F) -> io::Result<Vec<IpAddr>>
where
    F: FnOnce() -> io::Result<Vec<IpAddr>> + Send + 'static,
{
    let permit = slots
        .try_acquire_owned()
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "too many name lookups"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        lookup()
    })
    .await
    .map_err(|_| io::Error::other("lookup task failed"))?
}

/// Resolves `host` and checks every answer. Any non-global address denies
/// the whole name; an empty answer, an error or a timeout is
/// `ResolveFailed`. Returns the checked addresses with `port` attached.
pub async fn resolve_checked(
    resolver: &dyn Resolver,
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, DenyReason> {
    let answer = tokio::time::timeout(RESOLVE_TIMEOUT, resolver.resolve(host))
        .await
        .map_err(|_| DenyReason::ResolveFailed)?
        .map_err(|_| DenyReason::ResolveFailed)?;
    check_addresses(&answer, port)
}

/// The address check on its own (pure).
pub fn check_addresses(answer: &[IpAddr], port: u16) -> Result<Vec<SocketAddr>, DenyReason> {
    if answer.is_empty() {
        return Err(DenyReason::ResolveFailed);
    }
    let mut out: Vec<SocketAddr> = Vec::new();
    for ip in answer {
        if blocked_range(*ip).is_some() {
            return Err(DenyReason::NonGlobalAddress);
        }
        let sa = SocketAddr::new(*ip, port);
        if !out.contains(&sa) && out.len() < MAX_ADDRS {
            out.push(sa);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;

    use super::*;

    /// Resolver with a fixed table; unknown names fail to resolve.
    #[derive(Default)]
    pub struct TableResolver(pub HashMap<String, Vec<IpAddr>>);

    impl TableResolver {
        pub fn with(entries: &[(&str, &[&str])]) -> TableResolver {
            TableResolver(
                entries
                    .iter()
                    .map(|(h, ips)| {
                        (
                            (*h).to_string(),
                            ips.iter().map(|i| i.parse().expect("test ip")).collect(),
                        )
                    })
                    .collect(),
            )
        }
    }

    impl Resolver for TableResolver {
        fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
            Box::pin(async move {
                self.0
                    .get(host)
                    .cloned()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such host"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TableResolver;
    use super::*;

    #[tokio::test]
    async fn lookups_are_capped_and_abandoned_ones_keep_their_slot() {
        let slots = Arc::new(Semaphore::new(MAX_LOOKUPS));
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
        let mut pending = Vec::new();
        for _ in 0..MAX_LOOKUPS {
            let rx = release_rx.clone();
            // The caller gives up at once (timeout): the thread is still busy.
            let fut = bounded_lookup(slots.clone(), move || {
                let _ = rx.lock().expect("lock").recv();
                Ok(vec![])
            });
            pending.push(tokio::spawn(fut));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        for p in &pending {
            p.abort();
        }
        let err = bounded_lookup(slots.clone(), || Ok(vec![]))
            .await
            .expect_err("saturated");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        for _ in 0..MAX_LOOKUPS {
            release_tx.send(()).expect("release");
        }
        for _ in 0..100 {
            if slots.available_permits() == MAX_LOOKUPS {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(slots.available_permits(), MAX_LOOKUPS);
        bounded_lookup(slots, || Ok(vec![]))
            .await
            .expect("a free slot works again");
    }

    #[tokio::test]
    async fn public_addresses_pass_with_the_port_attached() {
        let r = TableResolver::with(&[("ok.example", &["93.184.216.34", "2606:2800:220:1::1"])]);
        let got = resolve_checked(&r, "ok.example", 443)
            .await
            .expect("allowed");
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|a| a.port() == 443));
    }

    #[tokio::test]
    async fn a_private_answer_is_a_rebinding_denial() {
        let r = TableResolver::with(&[
            ("rebind.example", &["127.0.0.1"]),
            ("meta.example", &["169.254.169.254"]),
            ("lan.example", &["192.168.1.10"]),
            ("v6.example", &["::1"]),
            ("mapped.example", &["::ffff:10.0.0.1"]),
            ("mixed.example", &["93.184.216.34", "10.0.0.5"]),
        ]);
        for h in [
            "rebind.example",
            "meta.example",
            "lan.example",
            "v6.example",
            "mapped.example",
            "mixed.example",
        ] {
            assert_eq!(
                resolve_checked(&r, h, 443).await,
                Err(DenyReason::NonGlobalAddress),
                "{h}"
            );
        }
    }

    #[tokio::test]
    async fn failures_are_resolve_failed() {
        let r = TableResolver::with(&[("empty.example", &[])]);
        assert_eq!(
            resolve_checked(&r, "nx.example", 443).await,
            Err(DenyReason::ResolveFailed)
        );
        assert_eq!(
            resolve_checked(&r, "empty.example", 443).await,
            Err(DenyReason::ResolveFailed)
        );
    }

    #[test]
    fn duplicates_collapse_and_answers_are_capped() {
        let many: Vec<IpAddr> = (1..=20)
            .map(|i| format!("93.184.216.{i}").parse().expect("ip"))
            .collect();
        assert_eq!(check_addresses(&many, 80).expect("ok").len(), MAX_ADDRS);
        let dup: Vec<IpAddr> = vec!["1.1.1.1".parse().expect("ip"); 3];
        assert_eq!(check_addresses(&dup, 80).expect("ok").len(), 1);
    }
}

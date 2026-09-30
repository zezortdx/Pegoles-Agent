//! Accept loop: one host connection at a time; a second one is refused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::session::run_session;
use crate::transport::Acceptor;
use crate::{policy_file, Config};

const ACCEPT_RETRY: Duration = Duration::from_millis(200);

/// Clears the "a session is active" flag even if the session panics.
struct ActiveGuard(Arc<AtomicBool>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Resets the policy file, then serves host connections forever.
pub fn serve<A: Acceptor>(acceptor: &A, cfg: &Config) {
    if let Err(e) = policy_file::reset(&cfg.policy_file) {
        log!("startup: cannot reset the policy file: {e}");
    }
    let active = Arc::new(AtomicBool::new(false));
    loop {
        let conn = match acceptor.accept() {
            Ok(c) => c,
            Err(e) => {
                log!("accept failed: {e}");
                thread::sleep(ACCEPT_RETRY);
                continue;
            }
        };
        if active.swap(true, Ordering::SeqCst) {
            log!("refused a second host connection");
            drop(conn);
            continue;
        }
        let guard = ActiveGuard(active.clone());
        let cfg = cfg.clone();
        let spawned = thread::Builder::new()
            .name("session".into())
            .spawn(move || {
                let _guard = guard;
                run_session(conn, &cfg);
            });
        if let Err(e) = spawned {
            // The closure (and its guard) was dropped: the flag is clear.
            log!("cannot start a session thread: {e}");
        }
    }
}

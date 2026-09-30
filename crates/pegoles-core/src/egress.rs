//! Internet access for the running task (docs/EGRESS.md), owned by Core.
//!
//! The computer has no network device. While a task the person started
//! with internet enabled (and confirmed natively) is running, Core asks
//! the backend for the one egress byte stream and serves the host proxy
//! (`pegoles-egress`) behind it on a dedicated tokio runtime. The session
//! lives exactly as long as the agent controls a running computer: every
//! path that ends or takes control from the agent run closes it (stop,
//! pause, reset, destroy, human takeover, task end, failure, app exit),
//! and dropping the registry does too.
//!
//! Decisions reach the UI as [`EgressDecision`]s (host, verdict, reason
//! code, bytes, time; the egress crate already strips paths' queries and
//! never carries headers or bodies). They pass a bounded drop-oldest
//! queue so a slow consumer can never stall a proxy task; what was dropped
//! is counted.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use pegoles_computer::EgressEndpoint;
use pegoles_egress::policy::parse_list;
use pegoles_egress::{AuditEvent, Decision, EgressSession, Mode};
use pegoles_protocol::{
    AgentEvent, ComputerState, EgressDecision, InternetAccess, InternetMode, TaskId,
};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, oneshot, Notify};

use crate::events::EventBus;
use crate::input::CancellationToken;
use crate::registry::ComputerRegistry;

/// Audit events waiting for the forwarder; beyond this the oldest go.
const QUEUE_CAP: usize = 256;
/// Decisions kept for the status payload (reload, late subscribers).
const RECENT_CAP: usize = 200;
/// How many of those the status carries.
pub const STATUS_RECENT: usize = 20;
/// How long `close` waits for the session task to finish: the stream is
/// dropped immediately either way, this only makes "closed" observable.
const CLOSE_WAIT: Duration = Duration::from_millis(500);
/// Budget for the guest to answer HELLO after the stream is connected.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Validate a task's internet setting (mode rules of docs/EGRESS.md) and
/// normalize it: `allowlist` needs 1..=32 valid domains (lowercased,
/// deduplicated), the other modes carry none. The error is a sentence for
/// the person.
pub fn validate_internet(access: &InternetAccess) -> Result<(InternetAccess, Mode), String> {
    match access.mode {
        InternetMode::Off => Ok((InternetAccess::off(), Mode::Off)),
        InternetMode::OpenWeb => Ok((
            InternetAccess {
                mode: InternetMode::OpenWeb,
                domains: Vec::new(),
            },
            Mode::OpenWeb,
        )),
        InternetMode::Allowlist => {
            let list = parse_list(access.domains.iter().map(String::as_str))
                .map_err(|e| format!("Internet access was not set: {e}."))?;
            let domains: Vec<String> = list.iter().map(|d| d.as_str().to_string()).collect();
            Ok((
                InternetAccess {
                    mode: InternetMode::Allowlist,
                    domains,
                },
                Mode::Allowlist(list),
            ))
        }
    }
}

/// What the UI shows about the current internet session.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EgressStatus {
    pub active: bool,
    pub task_id: Option<String>,
    pub mode: InternetMode,
    pub domains: Vec<String>,
    pub allowed: u64,
    pub blocked: u64,
    /// Decisions dropped because the UI could not keep up.
    pub dropped: u64,
    /// Newest last.
    pub recent: Vec<EgressDecision>,
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Adopt the accepted endpoint into tokio. Must run inside the runtime.
fn adopt(endpoint: EgressEndpoint) -> std::io::Result<Box<dyn Stream>> {
    #[cfg(unix)]
    {
        let stream = endpoint.into_unix();
        stream.set_nonblocking(true)?;
        Ok(Box::new(tokio::net::UnixStream::from_std(stream)?))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        let raw = endpoint.into_pipe().into_raw_handle();
        // SAFETY: the handle was opened for overlapped I/O and is owned by
        // us (`into_raw_handle` forgot the wrapper); tokio takes it over.
        let pipe =
            unsafe { tokio::net::windows::named_pipe::NamedPipeServer::from_raw_handle(raw)? };
        Ok(Box::new(pipe))
    }
}

/// Bounded queue between the proxy tasks (which must never block) and the
/// forwarder: drop-oldest with a counter.
struct Queue {
    events: Mutex<VecDeque<AuditEvent>>,
    dropped: AtomicU64,
    wake: Notify,
}

impl Queue {
    fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
            wake: Notify::new(),
        }
    }

    fn push(&self, event: AuditEvent) {
        {
            let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
            if q.len() >= QUEUE_CAP {
                q.pop_front();
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            q.push_back(event);
        }
        self.wake.notify_one();
    }

    fn drain(&self) -> (Vec<AuditEvent>, u64) {
        let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        (
            q.drain(..).collect(),
            self.dropped.swap(0, Ordering::Relaxed),
        )
    }
}

struct Shared {
    bus: EventBus,
    notices: broadcast::Sender<EgressDecision>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    generation: u64,
    active: Option<Active>,
    recent: VecDeque<EgressDecision>,
    allowed: u64,
    blocked: u64,
    dropped: u64,
    /// The session ended by itself (guest closed it, protocol violation):
    /// the backend's stream still needs closing from the registry.
    ended_unreported: bool,
}

struct Active {
    generation: u64,
    task: TaskId,
    access: InternetAccess,
    kill: Option<oneshot::Sender<()>>,
    done: mpsc::Receiver<()>,
    reason: Arc<Mutex<Option<String>>>,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The answer to "did the channel come up?", waited on with Core unlocked.
pub struct EgressHandshake {
    rx: mpsc::Receiver<Result<(), String>>,
}

impl EgressHandshake {
    /// Block (in short slices, honoring `cancel`) until the guest has
    /// acknowledged, the channel failed, or `timeout` passed.
    pub fn wait(&self, cancel: &CancellationToken, timeout: Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if cancel.is_cancelled() {
                return Err("the task was stopped".to_string());
            }
            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() >= deadline {
                        return Err("the computer did not answer in time".to_string());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("the internet channel closed before it was ready".to_string())
                }
            }
        }
    }
}

/// An internet session being opened: the backend has been told to connect
/// the guest's forwarder; waiting for that needs no lock.
pub struct EgressOpening {
    task: TaskId,
    access: InternetAccess,
    mode: Mode,
    ticket: u64,
    pending: Option<Box<dyn pegoles_computer::PendingEgressOpen>>,
}

impl EgressOpening {
    /// Step 2, run with Core unlocked: wait for the helper's answer and the
    /// guest's connection (both bounded, see the egress timeouts).
    pub fn wait(&mut self) -> Result<EgressEndpoint, String> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| "the internet channel was already waited for".to_string())?;
        pending
            .wait()
            .map_err(|e| format!("the computer could not open its internet channel: {e}"))
    }
}

/// Owns the egress runtime and the one session a computer can have.
pub struct EgressController {
    shared: Arc<Shared>,
    runtime: Option<tokio::runtime::Runtime>,
    /// Bumped by every prepare and every close; an opening may install
    /// its stream only while its ticket is still the current one.
    ticket: u64,
}

impl EgressController {
    pub fn new(bus: EventBus) -> Self {
        let (notices, _) = broadcast::channel(256);
        Self {
            shared: Arc::new(Shared {
                bus,
                notices,
                state: Mutex::new(State::default()),
            }),
            runtime: None,
            ticket: 0,
        }
    }

    fn next_ticket(&mut self) -> u64 {
        self.ticket += 1;
        self.ticket
    }

    fn ticket(&self) -> u64 {
        self.ticket
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EgressDecision> {
        self.shared.notices.subscribe()
    }

    pub fn is_active(&self) -> bool {
        lock(&self.shared.state).active.is_some()
    }

    /// The session ended by itself since the last call.
    fn take_ended(&self) -> bool {
        std::mem::take(&mut lock(&self.shared.state).ended_unreported)
    }

    pub fn status(&self) -> EgressStatus {
        let st = lock(&self.shared.state);
        let skip = st.recent.len().saturating_sub(STATUS_RECENT);
        EgressStatus {
            active: st.active.is_some(),
            task_id: st.active.as_ref().map(|a| a.task.to_string()),
            mode: st
                .active
                .as_ref()
                .map_or(InternetMode::Off, |a| a.access.mode),
            domains: st
                .active
                .as_ref()
                .map(|a| a.access.domains.clone())
                .unwrap_or_default(),
            allowed: st.allowed,
            blocked: st.blocked,
            dropped: st.dropped,
            recent: st.recent.iter().skip(skip).cloned().collect(),
        }
    }

    fn runtime(&mut self) -> Result<&tokio::runtime::Runtime, String> {
        if self.runtime.is_none() {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("pegoles-egress")
                .enable_all()
                .build()
                .map_err(|e| format!("the internet proxy could not start: {e}"))?;
            self.runtime = Some(rt);
        }
        self.runtime
            .as_ref()
            .ok_or_else(|| "the internet proxy could not start".to_string())
    }

    /// Serve `endpoint` for `task`. Returns at once; the handshake result
    /// arrives on the returned handle.
    fn start(
        &mut self,
        task: TaskId,
        access: InternetAccess,
        mode: Mode,
        endpoint: EgressEndpoint,
    ) -> Result<EgressHandshake, String> {
        let shared = self.shared.clone();
        let handle = self.runtime()?.handle().clone();
        let (hs_tx, hs_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let (kill_tx, kill_rx) = oneshot::channel();
        let reason = Arc::new(Mutex::new(None));
        let generation = {
            let mut st = lock(&shared.state);
            st.generation += 1;
            st.recent.clear();
            (st.allowed, st.blocked, st.dropped) = (0, 0, 0);
            st.ended_unreported = false;
            st.active = Some(Active {
                generation: st.generation,
                task,
                access: access.clone(),
                kill: Some(kill_tx),
                done: done_rx,
                reason: reason.clone(),
            });
            st.generation
        };
        let job = Job {
            shared,
            generation,
            task,
            access,
            mode,
            kill_rx,
            hs_tx,
            done_tx,
            reason,
        };
        handle.spawn(run_session(job, endpoint));
        Ok(EgressHandshake { rx: hs_rx })
    }

    /// Kill switch: end the session if there is one. Returns whether a
    /// session existed. The stream is dropped before this returns (bounded
    /// wait), which cuts every guest connection.
    pub fn close(&self, reason: &str) -> bool {
        self.shared.kill(reason)
    }

    /// A handle that cuts the session without Core's lock (Stop, pause,
    /// takeover and app exit use it so they never queue behind a guest
    /// call). The backend's own stream is closed by the registry on its
    /// next pump.
    pub fn kill_handle(&self) -> EgressKill {
        EgressKill(self.shared.clone())
    }
}

impl Shared {
    fn kill(&self, reason: &str) -> bool {
        let active = lock(&self.state).active.take();
        let Some(mut active) = active else {
            return false;
        };
        *active.reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason.to_string());
        if let Some(kill) = active.kill.take() {
            let _ = kill.send(());
        }
        let _ = active.done.recv_timeout(CLOSE_WAIT);
        true
    }
}

/// Cuts the current internet session from anywhere (see
/// [`EgressController::kill_handle`]).
#[derive(Clone)]
pub struct EgressKill(Arc<Shared>);

impl EgressKill {
    /// Idempotent. Returns whether a session was cut.
    pub fn kill(&self, reason: &str) -> bool {
        let cut = self.0.kill(reason);
        if cut {
            lock(&self.0.state).ended_unreported = true;
        }
        cut
    }
}

impl Drop for EgressController {
    fn drop(&mut self) {
        self.close("Pegoles is closing");
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

struct Job {
    shared: Arc<Shared>,
    generation: u64,
    task: TaskId,
    access: InternetAccess,
    mode: Mode,
    kill_rx: oneshot::Receiver<()>,
    hs_tx: mpsc::Sender<Result<(), String>>,
    done_tx: mpsc::Sender<()>,
    reason: Arc<Mutex<Option<String>>>,
}

/// Why a session ended on its own, in a sentence.
fn natural_end(end: &pegoles_egress::SessionEnd) -> &'static str {
    match end {
        pegoles_egress::SessionEnd::Shutdown => "the internet channel was shut down",
        pegoles_egress::SessionEnd::PeerClosed => "the computer closed its internet channel",
        pegoles_egress::SessionEnd::Protocol(_) => {
            "the computer broke the internet channel rules, so it was cut"
        }
        pegoles_egress::SessionEnd::Io(_) => "the internet channel failed",
    }
}

async fn run_session(mut job: Job, endpoint: EgressEndpoint) {
    let queue = Arc::new(Queue::new());
    let sink_queue = queue.clone();
    let sink: pegoles_egress::AuditSink = Arc::new(move |e| sink_queue.push(e));
    let mode = std::mem::replace(&mut job.mode, Mode::Off);
    let outcome = async move {
        let stream = adopt(endpoint).map_err(|e| format!("the internet channel failed: {e}"))?;
        EgressSession::start(stream, mode, sink)
            .await
            .map_err(|e| format!("the internet channel could not be set up: {e}"))
    };
    let session = tokio::select! {
        r = outcome => r,
        _ = &mut job.kill_rx => Err("the task's internet access was closed".to_string()),
    };
    let session = match session {
        Ok(s) => s,
        Err(message) => {
            let _ = job.hs_tx.send(Err(message));
            clear_if_current(&job, false);
            let _ = job.done_tx.send(());
            return;
        }
    };
    let _ = job.hs_tx.send(Ok(()));
    job.shared.bus.publish(AgentEvent::InternetOpened {
        task_id: job.task,
        mode: job.access.mode,
        domains: job.access.domains.clone(),
        at: Utc::now(),
    });
    let mut natural = None;
    loop {
        tokio::select! {
            _ = &mut job.kill_rx => break,
            end = session.closed() => {
                natural = Some(natural_end(&end));
                break;
            }
            _ = queue.wake.notified() => forward(&job, &queue),
        }
    }
    session.shutdown().await;
    forward(&job, &queue);
    if natural.is_some() {
        clear_if_current(&job, true);
    }
    let reason = job
        .reason
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .or_else(|| natural.map(str::to_string))
        .unwrap_or_else(|| "the task's internet access ended".to_string());
    job.shared.bus.publish(AgentEvent::InternetClosed {
        task_id: job.task,
        reason,
        at: Utc::now(),
    });
    let _ = job.done_tx.send(());
}

/// The session is gone on its own: forget it (unless `close` already did
/// or a newer session replaced it).
fn clear_if_current(job: &Job, flag_backend: bool) {
    let mut st = lock(&job.shared.state);
    if st
        .active
        .as_ref()
        .is_some_and(|a| a.generation == job.generation)
    {
        st.active = None;
        st.ended_unreported = flag_backend;
    }
}

fn forward(job: &Job, queue: &Queue) {
    let (events, dropped) = queue.drain();
    if events.is_empty() && dropped == 0 {
        return;
    }
    let mut st = lock(&job.shared.state);
    if st.generation != job.generation {
        return;
    }
    st.dropped += dropped;
    let mut dropped_before = dropped;
    for e in events {
        let allowed = e.decision == Decision::Allow;
        if allowed {
            st.allowed += 1;
        } else {
            st.blocked += 1;
        }
        let decision = EgressDecision {
            task_id: job.task,
            host: e.host.clone(),
            allowed,
            reason: e.reason_code().to_string(),
            bytes: e.bytes,
            dropped_before: std::mem::take(&mut dropped_before),
            at: DateTime::<Utc>::from(e.time),
        };
        if st.recent.len() >= RECENT_CAP {
            st.recent.pop_front();
        }
        st.recent.push_back(decision.clone());
        let _ = job.shared.notices.send(decision);
    }
}

impl ComputerRegistry {
    /// Open the task's internet session in one step (the caller has the
    /// person's consent). This waits for the helper with the caller's lock
    /// held, so UI and agent paths use [`egress_prepare`](Self::egress_prepare),
    /// [`EgressOpening::wait`] and [`egress_install`](Self::egress_install)
    /// instead. Returns once the backend's stream is connected; wait for the
    /// handshake on the handle with Core unlocked.
    pub fn egress_begin(
        &mut self,
        task: TaskId,
        access: &InternetAccess,
    ) -> Result<EgressHandshake, String> {
        let mut opening = self.egress_prepare(task, access)?;
        let endpoint = opening.wait();
        self.egress_install(opening, endpoint)
    }

    /// Step 1 of opening the task's internet session, under Core's lock and
    /// quick: checks the task may have internet now, closes any previous
    /// session and has the backend create its endpoint and send the
    /// helper's command. Nothing here waits for the helper or the guest.
    pub fn egress_prepare(
        &mut self,
        task: TaskId,
        access: &InternetAccess,
    ) -> Result<EgressOpening, String> {
        let (access, mode) = validate_internet(access)?;
        if access.is_off() {
            return Err("internet is off for this task".to_string());
        }
        if self.state() != Some(ComputerState::Running) || !self.agent_controls() {
            return Err("internet can only open while the agent runs the task".to_string());
        }
        self.egress_close("replaced by a new session");
        // An opening still in flight owns the backend's stream without a
        // session to show for it: a new opening replaces it (idempotent).
        self.close_backend_egress();
        let backend = self
            .backend
            .as_deref_mut()
            .ok_or_else(|| "there is no computer".to_string())?;
        let pending = backend
            .begin_open_egress()
            .map_err(|e| format!("the computer could not open its internet channel: {e}"))?;
        Ok(EgressOpening {
            task,
            access,
            mode,
            ticket: self.egress.next_ticket(),
            pending: Some(pending),
        })
    }

    /// Step 3, under Core's lock and quick: serve the connected stream, but
    /// only if the task still owns the running computer and nothing
    /// replaced or closed this opening meanwhile; otherwise the stream is
    /// dropped (the guest sees it close). `endpoint` is what
    /// [`EgressOpening::wait`] returned.
    pub fn egress_install(
        &mut self,
        opening: EgressOpening,
        endpoint: Result<EgressEndpoint, String>,
    ) -> Result<EgressHandshake, String> {
        let endpoint = match endpoint {
            Ok(e) => e,
            Err(e) => {
                // The backend undid its own open; only make sure of it.
                if opening.ticket == self.egress.ticket() {
                    self.close_backend_egress();
                }
                return Err(e);
            }
        };
        if opening.ticket != self.egress.ticket() {
            // Closed or replaced meanwhile: whoever did that owns the
            // backend's stream now.
            drop(endpoint);
            return Err("the internet channel was closed while it opened".to_string());
        }
        if self.state() != Some(ComputerState::Running) || !self.agent_controls() {
            drop(endpoint);
            self.close_backend_egress();
            return Err("the task no longer controls the computer".to_string());
        }
        match self
            .egress
            .start(opening.task, opening.access, opening.mode, endpoint)
        {
            Ok(handshake) => Ok(handshake),
            Err(e) => {
                self.close_backend_egress();
                Err(e)
            }
        }
    }

    /// Kill switch (idempotent): end the session and close the backend's
    /// stream. Called by every path that ends or takes control from the
    /// agent run.
    pub fn egress_close(&mut self, reason: &str) {
        // An opening still waiting for its stream is void too.
        self.egress.next_ticket();
        let cut = self.egress.close(reason);
        // A cut made through the lock-free handle left the backend stream
        // for us to close.
        if self.egress.take_ended() || cut {
            self.close_backend_egress();
        }
    }

    fn close_backend_egress(&mut self) {
        if let Some(backend) = self.backend.as_deref_mut() {
            let _ = backend.close_egress();
        }
    }

    /// Called from `pump`: a session only lives while the agent controls
    /// a running computer, and a session the guest ended needs the
    /// backend stream closed too.
    pub(crate) fn egress_reconcile(&mut self) {
        if self.egress.take_ended() {
            self.close_backend_egress();
        }
        if self.egress.is_active()
            && (self.state() != Some(ComputerState::Running) || !self.agent_controls())
        {
            self.egress_close("the agent no longer controls the computer");
        }
    }

    /// Lock-free kill switch for the current session.
    pub fn egress_kill_handle(&self) -> EgressKill {
        self.egress.kill_handle()
    }

    pub fn egress_status(&self) -> EgressStatus {
        self.egress.status()
    }

    /// Live internet decisions (host, verdict, reason code, bytes, time).
    pub fn subscribe_egress(&self) -> broadcast::Receiver<EgressDecision> {
        self.egress.subscribe()
    }
}

#[cfg(all(test, unix))]
mod tests;

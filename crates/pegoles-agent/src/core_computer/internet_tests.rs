//! The run's internet step: confirm natively, open after the agent holds
//! the computer, stay offline (and say so) on any refusal.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pegoles_computer::{EgressPeerHandle, MockComputerBackend};
use pegoles_core::{BackendKind, CancellationToken, ComputerRegistry, EventBus, TaskManager};
use pegoles_egress_proto::{Conn, Decoder, Role};
use pegoles_protocol::{AgentEvent, InternetAccess, InternetMode, TaskId, TaskStatus};
use tokio::sync::broadcast;

use super::*;
use crate::runner::AgentComputer;

struct Shared(Arc<Mutex<(ComputerRegistry, TaskManager)>>);

impl CoreAccess for Shared {
    fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let (r, t) = &mut *guard;
        f(r, t)
    }
}

struct Rig {
    core: Arc<Mutex<(ComputerRegistry, TaskManager)>>,
    bus: EventBus,
    events: broadcast::Receiver<AgentEvent>,
    peer: EgressPeerHandle,
    task: TaskId,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    rig_with_open_delay(Duration::ZERO)
}

fn rig_with_open_delay(open_delay: Duration) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let bus = EventBus::new();
    let events = bus.subscribe();
    let mut registry =
        ComputerRegistry::with_dirs(bus.clone(), BackendKind::Mock, dir.path().to_path_buf());
    let mut mock = MockComputerBackend::new();
    mock.set_input_enabled(true);
    mock.set_egress_open_delay(open_delay);
    let peer = mock.egress_peer_handle();
    registry.create_default_on(Box::new(mock)).unwrap();
    let mut tasks = TaskManager::new(bus.clone());
    let task = tasks.submit_task("browse").unwrap().id;
    Rig {
        core: Arc::new(Mutex::new((registry, tasks))),
        bus,
        events,
        peer,
        task,
        _dir: dir,
    }
}

fn web() -> InternetAccess {
    InternetAccess {
        mode: InternetMode::Allowlist,
        domains: vec!["example.com".into()],
    }
}

fn computer(rig: &Rig, access: InternetAccess, confirm: InternetConfirm) -> CoreComputer<Shared> {
    let mut c =
        CoreComputer::new(Shared(rig.core.clone()), rig.bus.clone()).with_internet(InternetPlan {
            task: rig.task,
            access,
            confirm,
        });
    c.prepare_timeout = Duration::from_secs(3);
    c.internet_timeout = Duration::from_millis(400);
    c
}

fn answer(result: Result<bool, String>, asked: Arc<AtomicUsize>) -> InternetConfirm {
    Arc::new(move |_| {
        asked.fetch_add(1, Ordering::SeqCst);
        result.clone()
    })
}

fn active(rig: &Rig) -> bool {
    rig.core.lock().unwrap().0.egress_status().active
}

fn events(rig: &mut Rig) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rig.events.try_recv() {
        out.push(e);
    }
    out
}

fn fake_guest(mut peer: UnixStream) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (mut conn, mut decoder) = (Conn::new(Role::Guest), Decoder::new());
        let (mut buf, mut frames) = ([0u8; 4096], Vec::new());
        while frames.is_empty() {
            match peer.read(&mut buf) {
                Ok(0) | Err(_) => return false,
                Ok(n) => decoder.decode_all(&buf[..n], &mut frames).unwrap(),
            }
        }
        conn.on_frame(&frames[0]).unwrap();
        peer.write_all(&conn.ack_frame().unwrap().to_bytes().unwrap())
            .unwrap();
        loop {
            match peer.read(&mut buf) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
}

fn offline_reason(events: &[AgentEvent]) -> Option<String> {
    events.iter().find_map(|e| match e {
        AgentEvent::InternetUnavailable { reason, .. } => Some(reason.clone()),
        _ => None,
    })
}

#[test]
fn off_never_asks_and_never_opens() {
    let mut rig = rig();
    let asked = Arc::new(AtomicUsize::new(0));
    let c = computer(&rig, InternetAccess::off(), answer(Ok(true), asked.clone()));
    c.prepare(&CancellationToken::new()).unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert!(!active(&rig));
    assert!(rig.peer.take().is_none());
    assert!(offline_reason(&events(&mut rig)).is_none());
}

#[test]
fn declined_consent_starts_the_task_offline_and_says_so() {
    let mut rig = rig();
    let asked = Arc::new(AtomicUsize::new(0));
    let c = computer(&rig, web(), answer(Ok(false), asked.clone()));
    c.prepare(&CancellationToken::new()).unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(!active(&rig));
    assert!(rig.peer.take().is_none(), "the backend was never asked");
    let evs = events(&mut rig);
    assert!(offline_reason(&evs).unwrap().contains("not confirmed"));
    assert!(evs.iter().any(|e| matches!(
        e, AgentEvent::AgentMessage { text, .. } if text.contains("continues offline")
    )));
}

#[test]
fn an_unanswerable_confirmation_also_fails_closed() {
    let mut rig = rig();
    let c = computer(
        &rig,
        web(),
        answer(Err("another confirmation is open".into()), Arc::default()),
    );
    c.prepare(&CancellationToken::new()).unwrap();
    assert!(!active(&rig));
    assert_eq!(
        offline_reason(&events(&mut rig)).as_deref(),
        Some("another confirmation is open")
    );
}

#[test]
fn an_invalid_setting_is_refused_before_asking() {
    let rig = rig();
    let asked = Arc::new(AtomicUsize::new(0));
    let bad = InternetAccess {
        mode: InternetMode::Allowlist,
        domains: vec!["not a domain".into()],
    };
    let c = computer(&rig, bad, answer(Ok(true), asked.clone()));
    c.prepare(&CancellationToken::new()).unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert!(!active(&rig));
}

#[test]
fn a_guest_that_does_not_answer_leaves_the_task_offline() {
    let mut rig = rig();
    let c = computer(&rig, web(), answer(Ok(true), Arc::default()));
    c.prepare(&CancellationToken::new()).unwrap();
    assert!(!active(&rig), "closed again after the failed handshake");
    assert!(offline_reason(&events(&mut rig))
        .unwrap()
        .contains("did not answer"));
}

#[test]
fn a_confirmed_task_goes_online_and_the_end_of_the_task_cuts_it() {
    let mut rig = rig();
    let c = computer(&rig, web(), answer(Ok(true), Arc::default()));
    let peer = rig.peer.clone();
    // The guest attaches as soon as the backend opens its stream.
    let guest = std::thread::spawn(move || loop {
        if let Some(p) = peer.take() {
            return fake_guest(p).join().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    });
    let mut c = c;
    c.internet_timeout = Duration::from_secs(5);
    c.prepare(&CancellationToken::new()).unwrap();
    assert!(active(&rig));
    let opened = events(&mut rig).into_iter().any(|e| {
        matches!(e, AgentEvent::InternetOpened { mode: InternetMode::Allowlist, task_id, .. } if task_id == rig.task)
    });
    assert!(opened);
    c.set_status(rig.task, TaskStatus::Running).unwrap();
    c.set_status(rig.task, TaskStatus::Failed).unwrap();
    assert!(!active(&rig), "a failed task loses the internet");
    assert!(guest.join().unwrap(), "the guest saw its stream cut");
}

#[test]
fn a_slow_internet_open_does_not_block_other_core_commands() {
    const OPEN_DELAY: Duration = Duration::from_millis(1500);
    let rig = rig_with_open_delay(OPEN_DELAY);
    let mut c = computer(&rig, web(), answer(Ok(true), Arc::default()));
    c.internet_timeout = Duration::from_secs(5);
    let peer = rig.peer.clone();
    let guest = std::thread::spawn(move || loop {
        if let Some(p) = peer.take() {
            return fake_guest(p).join().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    });
    let opening = std::thread::spawn(move || {
        c.prepare(&CancellationToken::new()).unwrap();
        c
    });
    // Let the open begin, then use Core like the UI and the agent do.
    std::thread::sleep(Duration::from_millis(400));
    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let _ = rig.core.lock().unwrap().0.egress_status();
        worst = worst.max(t.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        worst < Duration::from_millis(300),
        "Core was blocked for {worst:?} by a slow internet open"
    );
    let c = opening.join().unwrap();
    assert!(active(&rig), "the slow open still completes");
    drop(c);
    rig.core.lock().unwrap().0.egress_close("test over");
    let _ = guest.join();
}

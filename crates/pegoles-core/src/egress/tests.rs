//! Egress lifecycle against the Mock backend (`egress_peer_handle`): the
//! guest end is a socketpair, so a fake forwarder can do the mux handshake
//! and observe the stream being cut.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pegoles_computer::{EgressPeerHandle, MockComputerBackend};
use pegoles_egress_proto::{Conn, Decoder, Frame, Role};
use pegoles_protocol::{AgentEvent, ControlOwner, InternetAccess, InternetMode, TaskId};
use tokio::sync::broadcast;

use super::*;
use crate::events::EventBus;
use crate::input::CancellationToken;
use crate::registry::ComputerRegistry;
use crate::BackendKind;

const WAIT: Duration = Duration::from_secs(5);

struct Rig {
    registry: ComputerRegistry,
    peer: EgressPeerHandle,
    events: broadcast::Receiver<AgentEvent>,
    _dir: tempfile::TempDir,
}

/// A running Mock computer whose agent session has begun.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let bus = EventBus::new();
    let events = bus.subscribe();
    let mut registry =
        ComputerRegistry::with_dirs(bus, BackendKind::Mock, dir.path().to_path_buf());
    let mock = MockComputerBackend::new();
    let peer = mock.egress_peer_handle();
    registry.create_default_on(Box::new(mock)).unwrap();
    registry.start().unwrap();
    registry.begin_agent_session(&mut Vec::new()).unwrap();
    Rig {
        registry,
        peer,
        events,
        _dir: dir,
    }
}

fn allowlist(domains: &[&str]) -> InternetAccess {
    InternetAccess {
        mode: InternetMode::Allowlist,
        domains: domains.iter().map(|d| d.to_string()).collect(),
    }
}

/// The guest forwarder: answers HELLO, then reports whether the host cut
/// the stream (EOF) before the timeout.
fn fake_guest(mut peer: UnixStream) -> JoinHandle<bool> {
    std::thread::spawn(move || {
        peer.set_read_timeout(Some(WAIT)).unwrap();
        let mut conn = Conn::new(Role::Guest);
        let mut decoder = Decoder::new();
        let mut buf = [0u8; 4096];
        let mut frames = Vec::new();
        while frames.is_empty() {
            let n = match peer.read(&mut buf) {
                Ok(0) | Err(_) => return false,
                Ok(n) => n,
            };
            decoder.decode_all(&buf[..n], &mut frames).unwrap();
        }
        assert!(matches!(frames[0], Frame::Hello { .. }));
        conn.on_frame(&frames[0]).unwrap();
        peer.write_all(&conn.ack_frame().unwrap().to_bytes().unwrap())
            .unwrap();
        loop {
            match peer.read(&mut buf) {
                Ok(0) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
}

/// Open a session for a fresh task and return the guest's verdict handle.
fn open(rig: &mut Rig, access: &InternetAccess) -> (TaskId, JoinHandle<bool>) {
    let task = TaskId::new();
    let handshake = rig.registry.egress_begin(task, access).unwrap();
    let guest = fake_guest(rig.peer.take().expect("the backend opened its stream"));
    handshake
        .wait(&CancellationToken::new(), WAIT)
        .expect("guest acknowledged");
    assert!(rig.registry.egress_status().active);
    (task, guest)
}

fn drain(rx: &mut broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

#[test]
fn an_opening_closed_while_it_waits_never_installs() {
    let mut rig = rig();
    let web = allowlist(&["example.com"]);
    // closed (e.g. Stop) between the quick step and the install
    let mut opening = rig.registry.egress_prepare(TaskId::new(), &web).unwrap();
    let endpoint = opening.wait();
    rig.registry.egress_close("the person pressed Stop");
    let err = rig
        .registry
        .egress_install(opening, endpoint)
        .err()
        .unwrap();
    assert!(err.contains("closed"), "{err}");
    assert!(!rig.registry.egress_status().active);

    // the agent lost the computer while it waited
    let mut opening = rig.registry.egress_prepare(TaskId::new(), &web).unwrap();
    let endpoint = opening.wait();
    rig.registry.end_agent_session(&mut Vec::new());
    let err = rig
        .registry
        .egress_install(opening, endpoint)
        .err()
        .unwrap();
    assert!(
        err.contains("closed") || err.contains("no longer controls"),
        "{err}"
    );
    assert!(!rig.registry.egress_status().active);
}

#[test]
fn a_newer_opening_supersedes_an_older_one() {
    let mut rig = rig();
    let web = allowlist(&["example.com"]);
    let mut first = rig.registry.egress_prepare(TaskId::new(), &web).unwrap();
    let endpoint = first.wait();
    // a second prepare replaces the first (the backend allows one stream)
    let _second = rig.registry.egress_prepare(TaskId::new(), &web).unwrap();
    let err = rig.registry.egress_install(first, endpoint).err().unwrap();
    assert!(err.contains("closed"), "{err}");
    assert!(!rig.registry.egress_status().active);
}

#[test]
fn off_never_opens_egress() {
    let mut rig = rig();
    let err = rig
        .registry
        .egress_begin(TaskId::new(), &InternetAccess::off())
        .err()
        .expect("off opens nothing");
    assert!(err.contains("off"));
    assert!(!rig.registry.egress_status().active);
    assert!(rig.peer.take().is_none(), "the backend was never asked");
}

#[test]
fn invalid_settings_are_refused_with_a_sentence_and_open_nothing() {
    let mut rig = rig();
    for bad in [
        allowlist(&[]),
        allowlist(&["https://example.com/path"]),
        allowlist(&["*.example.com"]),
        allowlist(&["10.0.0.1"]),
    ] {
        let err = rig
            .registry
            .egress_begin(TaskId::new(), &bad)
            .err()
            .expect("invalid");
        assert!(err.starts_with("Internet access was not set"), "{err}");
    }
    let many: Vec<String> = (0..33).map(|i| format!("site{i}.example.com")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(validate_internet(&allowlist(&refs)).is_err());
    assert!(rig.peer.take().is_none());
}

#[test]
fn settings_are_normalized_per_mode() {
    let (access, _) = validate_internet(&allowlist(&[" Example.COM ", "example.com"])).unwrap();
    assert_eq!(access.domains, ["example.com"]);
    let open = InternetAccess {
        mode: InternetMode::OpenWeb,
        domains: vec!["ignored.example".into()],
    };
    assert!(validate_internet(&open).unwrap().0.domains.is_empty());
    let off = InternetAccess {
        mode: InternetMode::Off,
        domains: vec!["ignored.example".into()],
    };
    assert_eq!(validate_internet(&off).unwrap().0, InternetAccess::off());
}

#[test]
fn a_session_opens_reports_and_closes_on_the_kill_switch() {
    let mut rig = rig();
    let (task, guest) = open(&mut rig, &allowlist(&["example.com"]));
    let status = rig.registry.egress_status();
    assert_eq!(status.mode, InternetMode::Allowlist);
    assert_eq!(status.domains, ["example.com"]);
    assert_eq!(status.task_id, Some(task.to_string()));
    rig.registry.egress_close("test stop");
    assert!(!rig.registry.egress_status().active);
    assert!(guest.join().unwrap(), "the guest saw the stream cut");
    let events = drain(&mut rig.events);
    assert!(events.iter().any(|e| matches!(
        e, AgentEvent::InternetOpened { task_id, mode: InternetMode::Allowlist, .. } if *task_id == task
    )));
    assert!(events.iter().any(|e| matches!(
        e, AgentEvent::InternetClosed { reason, .. } if reason == "test stop"
    )));
}

type Loss = (&'static str, fn(&mut Rig));

#[test]
fn every_path_that_ends_or_takes_control_closes_the_session() {
    let paths: [Loss; 7] = [
        ("stop", |r| {
            r.registry.stop().unwrap();
        }),
        ("pause", |r| {
            r.registry.pause().unwrap();
        }),
        // The kill switch runs before the backend acts (the Mock refuses
        // reset/destroy while running; the session must be gone anyway).
        ("reset", |r| {
            let _ = r.registry.reset();
        }),
        ("destroy", |r| {
            let _ = r.registry.destroy();
        }),
        ("takeover / stop button", |r| {
            r.registry.cancel_agent_input("human takeover")
        }),
        ("task end", |r| {
            r.registry.end_agent_session(&mut Vec::new())
        }),
        ("control lost (pump reconcile)", |r| {
            r.registry.display.control = ControlOwner::User;
            r.registry.pump();
        }),
    ];
    for (name, lose) in paths {
        let mut rig = rig();
        let (_, guest) = open(
            &mut rig,
            &InternetAccess {
                mode: InternetMode::OpenWeb,
                domains: vec![],
            },
        );
        lose(&mut rig);
        assert!(!rig.registry.egress_status().active, "{name}: still active");
        assert!(guest.join().unwrap(), "{name}: the guest was not cut off");
        assert!(
            rig.peer.take().is_none(),
            "{name}: backend stream left open"
        );
    }
}

#[test]
fn dropping_the_registry_closes_the_session() {
    let mut rig = rig();
    let (_, guest) = open(&mut rig, &allowlist(&["example.com"]));
    drop(rig.registry);
    assert!(guest.join().unwrap(), "app exit cuts the stream");
}

#[test]
fn a_session_the_guest_closes_is_cleaned_up_on_the_next_pump() {
    let mut rig = rig();
    let task = TaskId::new();
    let handshake = rig
        .registry
        .egress_begin(task, &allowlist(&["example.com"]))
        .unwrap();
    let mut peer = rig.peer.take().unwrap();
    let mut conn = Conn::new(Role::Guest);
    let mut decoder = Decoder::new();
    peer.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = [0u8; 4096];
    let mut frames = Vec::new();
    while frames.is_empty() {
        let n = peer.read(&mut buf).unwrap();
        decoder.decode_all(&buf[..n], &mut frames).unwrap();
    }
    conn.on_frame(&frames[0]).unwrap();
    peer.write_all(&conn.ack_frame().unwrap().to_bytes().unwrap())
        .unwrap();
    handshake.wait(&CancellationToken::new(), WAIT).unwrap();
    drop(peer);
    let deadline = Instant::now() + WAIT;
    while rig.registry.egress_status().active && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!rig.registry.egress_status().active);
    rig.registry.pump();
    assert!(
        rig.registry
            .egress_begin(TaskId::new(), &allowlist(&["example.com"]))
            .is_ok(),
        "the backend stream was closed, so a new session can open"
    );
}

#[test]
fn a_guest_that_never_answers_leaves_the_task_offline() {
    let mut rig = rig();
    let handshake = rig
        .registry
        .egress_begin(TaskId::new(), &allowlist(&["example.com"]))
        .unwrap();
    let _silent = rig.peer.take().unwrap();
    let err = handshake
        .wait(&CancellationToken::new(), Duration::from_millis(200))
        .unwrap_err();
    assert!(err.contains("did not answer"), "{err}");
    rig.registry.egress_close("failed");
    assert!(!rig.registry.egress_status().active);
}

#[test]
fn a_stopped_task_does_not_wait_for_the_handshake() {
    let mut rig = rig();
    let handshake = rig
        .registry
        .egress_begin(TaskId::new(), &allowlist(&["example.com"]))
        .unwrap();
    let _silent = rig.peer.take().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(handshake.wait(&cancel, WAIT).is_err());
    rig.registry.egress_close("stopped");
}

#[test]
fn internet_only_opens_while_the_agent_controls_a_running_computer() {
    let mut rig = rig();
    rig.registry.end_agent_session(&mut Vec::new());
    let err = rig
        .registry
        .egress_begin(TaskId::new(), &allowlist(&["example.com"]))
        .err()
        .unwrap();
    assert!(err.contains("agent"), "{err}");
    assert!(rig.peer.take().is_none());
}

fn audit_event(host: &str, decision: Decision, path: &str) -> AuditEvent {
    AuditEvent::new(
        pegoles_egress::policy::ModeKind::OpenWeb,
        host,
        path,
        decision,
        12,
    )
}

#[tokio::test]
async fn audit_events_reach_the_stream_without_paths_or_queries() {
    // The forwarder, driven directly: events the proxy produces come out
    // as decisions carrying host, verdict, reason code, bytes and time.
    let controller = EgressController::new(EventBus::new());
    let mut rx = controller.subscribe();
    let queue = Queue::new();
    let (kill_tx, kill_rx) = oneshot::channel();
    let (hs_tx, _hs_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let task = TaskId::new();
    {
        let mut st = lock(&controller.shared.state);
        st.generation = 1;
    }
    let job = Job {
        shared: controller.shared.clone(),
        generation: 1,
        task,
        access: InternetAccess::off(),
        mode: Mode::Off,
        kill_rx,
        hs_tx,
        done_tx,
        reason: Arc::new(Mutex::new(None)),
    };
    drop(kill_tx);
    queue.push(audit_event(
        "ok.example.com",
        Decision::Allow,
        "/a?token=secret",
    ));
    queue.push(audit_event(
        "bad.example.net",
        Decision::Deny(pegoles_egress::DenyReason::ThreatMalware),
        "/x",
    ));
    forward(&job, &queue);
    let first = rx.recv().await.unwrap();
    let second = rx.recv().await.unwrap();
    assert!(first.allowed && first.reason == "allowed" && first.host == "ok.example.com");
    assert!(!second.allowed && second.reason == "threat_malware");
    assert_eq!(first.task_id, task);
    assert_eq!(first.bytes, 12);
    let wire = serde_json::to_string(&first).unwrap();
    assert!(!wire.contains("secret") && !wire.contains("/a"), "{wire}");
    let status = controller.status();
    assert_eq!((status.allowed, status.blocked), (1, 1));
    assert_eq!(status.recent.len(), 2);
}

#[test]
fn a_slow_consumer_loses_the_oldest_and_the_loss_is_counted() {
    let queue = Queue::new();
    for i in 0..(QUEUE_CAP + 10) {
        queue.push(audit_event(
            &format!("h{i}.example.com"),
            Decision::Allow,
            "/",
        ));
    }
    let (events, dropped) = queue.drain();
    assert_eq!(events.len(), QUEUE_CAP);
    assert_eq!(dropped, 10);
    assert_eq!(events[0].host, "h10.example.com", "the oldest were dropped");
}

#[test]
fn the_lock_free_kill_handle_cuts_the_session_and_the_next_pump_closes_the_backend() {
    let mut rig = rig();
    let (_, guest) = open(&mut rig, &allowlist(&["example.com"]));
    let handle = rig.registry.egress_kill_handle();
    assert!(handle.kill("Stop pressed"));
    assert!(!handle.kill("again"), "idempotent");
    assert!(guest.join().unwrap(), "the guest saw the stream cut");
    assert!(!rig.registry.egress_status().active);
    rig.registry.pump();
    assert!(
        rig.registry
            .egress_begin(TaskId::new(), &allowlist(&["example.com"]))
            .is_ok(),
        "the backend stream was closed by the pump"
    );
}

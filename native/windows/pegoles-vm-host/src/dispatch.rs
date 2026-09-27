//! The JSONL command set, platform-free and unit-tested on every OS. The
//! Windows specifics sit behind [`Machine`].

use std::collections::HashMap;
use std::sync::Arc;

use pegoles_computer::vmhost_proto::{
    CreateParams, HostCommand, HostError, HostErrorCode, HostEvent, HostRequest, HostResponse,
    VmState,
};

use crate::Out;

/// What the helper needs from Windows (broker + guest socket + serial).
pub trait Machine {
    /// Create the compute system through the broker and start it; open the
    /// serial pipe and begin connecting to the guest runtime.
    fn boot(&mut self, params: &CreateParams) -> Result<(), String>;
    fn pause(&mut self, id: &str) -> Result<(), String>;
    fn resume(&mut self, id: &str) -> Result<(), String>;
    /// Graceful stop (terminate if the guest does not cooperate); the
    /// compute system and its links are gone afterwards.
    fn halt(&mut self, id: &str) -> Result<(), String>;
    fn guest_send(&mut self, id: &str, payload: &str) -> Result<(), String>;
    fn guest_connected(&self, id: &str) -> bool;
    /// Drop the guest connection (the link reconnects on its own).
    fn guest_kick(&mut self, id: &str);
    /// Stop everything (stdin closed).
    fn shutdown(&mut self);
}

/// Off Windows: every VM operation fails closed, explicitly.
#[cfg(any(not(windows), test))]
pub struct Unavailable;

#[cfg(any(not(windows), test))]
impl Machine for Unavailable {
    fn boot(&mut self, _: &CreateParams) -> Result<(), String> {
        Err("Pegoles' Windows computer runs on Windows only".into())
    }
    fn pause(&mut self, _: &str) -> Result<(), String> {
        Err("not on Windows".into())
    }
    fn resume(&mut self, _: &str) -> Result<(), String> {
        Err("not on Windows".into())
    }
    fn halt(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn guest_send(&mut self, _: &str, _: &str) -> Result<(), String> {
        Err("no guest connection".into())
    }
    fn guest_connected(&self, _: &str) -> bool {
        false
    }
    fn guest_kick(&mut self, _: &str) {}
    fn shutdown(&mut self) {}
}

struct Entry {
    params: CreateParams,
    state: VmState,
}

pub struct Host<M: Machine> {
    machine: M,
    out: Arc<Out>,
    computers: HashMap<String, Entry>,
}

fn ok(id: u64, state: Option<VmState>) -> HostResponse {
    HostResponse {
        id,
        ok: true,
        state,
        version: None,
        connected: None,
        error: None,
    }
}

fn fail(id: u64, code: HostErrorCode, message: impl Into<String>) -> HostResponse {
    HostResponse {
        id,
        ok: false,
        state: None,
        version: None,
        connected: None,
        error: Some(HostError {
            code,
            message: message.into(),
        }),
    }
}

/// The engine's params, checked before anything is asked of the broker.
pub fn validate(params: &CreateParams) -> Result<(), String> {
    if !pegoles_broker_proto::is_uuid(&params.computer_id) {
        return Err("computer_id must be a lowercase UUID".into());
    }
    if !params.disk_path.to_ascii_lowercase().ends_with(".vhdx") {
        return Err(format!(
            "the Windows disk must be .vhdx, got {}",
            params.disk_path
        ));
    }
    if !(pegoles_broker_proto::MIN_VCPUS..=pegoles_broker_proto::MAX_VCPUS).contains(&params.vcpus)
    {
        return Err("vcpus out of range".into());
    }
    if !(pegoles_broker_proto::MIN_MEMORY_MB..=pegoles_broker_proto::MAX_MEMORY_MB)
        .contains(&params.memory_mb)
    {
        return Err("memory out of range".into());
    }
    if params.seed_iso_path.is_some() {
        return Err("seed ISOs are an image-builder step, not supported on Windows".into());
    }
    Ok(())
}

impl<M: Machine> Host<M> {
    pub fn new(machine: M, out: Arc<Out>) -> Self {
        Self {
            machine,
            out,
            computers: HashMap::new(),
        }
    }

    fn event_state(&self, id: &str, state: VmState) {
        self.out.line(&HostEvent::VmStateChanged {
            computer_id: id.to_string(),
            state,
        });
    }

    pub fn handle_line(&mut self, line: &str) -> HostResponse {
        match serde_json::from_str::<HostRequest>(line) {
            Ok(request) => self.handle(request),
            Err(_) => {
                let id = serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|v| v.get("id").and_then(|i| i.as_u64()))
                    .unwrap_or(0);
                fail(
                    id,
                    HostErrorCode::UnknownCommand,
                    "malformed or unknown request",
                )
            }
        }
    }

    pub fn handle(&mut self, request: HostRequest) -> HostResponse {
        let id = request.id;
        match request.command {
            HostCommand::Version => HostResponse {
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                ..ok(id, None)
            },
            HostCommand::Validate { params } => match validate(&params) {
                Ok(()) => ok(id, Some(VmState::Stopped)),
                Err(e) => fail(id, HostErrorCode::ValidationFailed, e),
            },
            HostCommand::Create { params } => {
                if self.computers.contains_key(&params.computer_id) {
                    return fail(
                        id,
                        HostErrorCode::AlreadyExists,
                        "computer already registered",
                    );
                }
                if let Err(e) = validate(&params) {
                    return fail(id, HostErrorCode::InvalidParams, e);
                }
                if !std::path::Path::new(&params.disk_path).is_file() {
                    return fail(
                        id,
                        HostErrorCode::InvalidParams,
                        "the computer's disk is missing",
                    );
                }
                self.computers.insert(
                    params.computer_id.clone(),
                    Entry {
                        params,
                        state: VmState::Stopped,
                    },
                );
                ok(id, Some(VmState::Stopped))
            }
            HostCommand::Start { computer_id } => {
                let Some(entry) = self.computers.get(&computer_id) else {
                    return fail(id, HostErrorCode::UnknownComputer, "no such computer");
                };
                if entry.state == VmState::Running {
                    return ok(id, Some(VmState::Running));
                }
                let params = entry.params.clone();
                match self.machine.boot(&params) {
                    Ok(()) => {
                        self.set(&computer_id, VmState::Running);
                        ok(id, Some(VmState::Running))
                    }
                    Err(e) => fail(id, HostErrorCode::StartFailed, e),
                }
            }
            HostCommand::Pause { computer_id } => {
                self.transition(id, &computer_id, VmState::Paused)
            }
            HostCommand::Resume { computer_id } => {
                self.transition(id, &computer_id, VmState::Running)
            }
            HostCommand::Stop { computer_id } => {
                let Some(entry) = self.computers.get(&computer_id) else {
                    return fail(id, HostErrorCode::UnknownComputer, "no such computer");
                };
                if entry.state == VmState::Stopped {
                    return ok(id, Some(VmState::Stopped));
                }
                match self.machine.halt(&computer_id) {
                    Ok(()) => {
                        self.set(&computer_id, VmState::Stopped);
                        ok(id, Some(VmState::Stopped))
                    }
                    Err(e) => fail(id, HostErrorCode::StopFailed, e),
                }
            }
            HostCommand::State { computer_id } => match self.computers.get(&computer_id) {
                Some(entry) => ok(id, Some(entry.state)),
                None => fail(id, HostErrorCode::UnknownComputer, "no such computer"),
            },
            HostCommand::Destroy { computer_id } => {
                if let Some(entry) = self.computers.remove(&computer_id) {
                    if entry.state != VmState::Stopped {
                        let _ = self.machine.halt(&computer_id);
                    }
                }
                ok(id, Some(VmState::Stopped))
            }
            HostCommand::GuestSend {
                computer_id,
                payload,
            } => {
                if !self.computers.contains_key(&computer_id) {
                    return fail(id, HostErrorCode::UnknownComputer, "no such computer");
                }
                if payload.contains('\n') || payload.len() > pegoles_guest_proto::MAX_FRAME_BYTES {
                    return fail(
                        id,
                        HostErrorCode::InvalidParams,
                        "one bounded frame per send",
                    );
                }
                match self.machine.guest_send(&computer_id, &payload) {
                    Ok(()) => ok(id, None),
                    Err(e) => fail(id, HostErrorCode::GuestUnavailable, e),
                }
            }
            HostCommand::GuestStatus { computer_id } => {
                if !self.computers.contains_key(&computer_id) {
                    return fail(id, HostErrorCode::UnknownComputer, "no such computer");
                }
                HostResponse {
                    connected: Some(self.machine.guest_connected(&computer_id)),
                    ..ok(id, None)
                }
            }
            HostCommand::GuestDisconnect { computer_id } => {
                if !self.computers.contains_key(&computer_id) {
                    return fail(id, HostErrorCode::UnknownComputer, "no such computer");
                }
                self.machine.guest_kick(&computer_id);
                ok(id, None)
            }
        }
    }

    fn set(&mut self, computer_id: &str, state: VmState) {
        if let Some(entry) = self.computers.get_mut(computer_id) {
            if entry.state != state {
                entry.state = state;
                self.event_state(computer_id, state);
            }
        }
    }

    fn transition(&mut self, id: u64, computer_id: &str, target: VmState) -> HostResponse {
        let Some(entry) = self.computers.get(computer_id) else {
            return fail(id, HostErrorCode::UnknownComputer, "no such computer");
        };
        let from = entry.state;
        let result = match (from, target) {
            (VmState::Running, VmState::Paused) => self.machine.pause(computer_id),
            (VmState::Paused, VmState::Running) => self.machine.resume(computer_id),
            (a, b) if a == b => Ok(()),
            _ => Err(format!("cannot go from {from:?} to {target:?}")),
        };
        match result {
            Ok(()) => {
                self.set(computer_id, target);
                ok(id, Some(target))
            }
            Err(e) if target == VmState::Paused => fail(id, HostErrorCode::PauseFailed, e),
            Err(e) => fail(id, HostErrorCode::ResumeFailed, e),
        }
    }

    pub fn shutdown(&mut self) {
        self.machine.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        fail_boot: bool,
    }

    impl Machine for Arc<Mutex<Fake>> {
        fn boot(&mut self, p: &CreateParams) -> Result<(), String> {
            let mut f = self.lock().unwrap();
            f.calls.push(format!("boot {}", p.computer_id));
            if f.fail_boot {
                Err("HcsCreateComputeSystem failed: HRESULT 0x80370102".into())
            } else {
                Ok(())
            }
        }
        fn pause(&mut self, id: &str) -> Result<(), String> {
            self.lock().unwrap().calls.push(format!("pause {id}"));
            Ok(())
        }
        fn resume(&mut self, id: &str) -> Result<(), String> {
            self.lock().unwrap().calls.push(format!("resume {id}"));
            Ok(())
        }
        fn halt(&mut self, id: &str) -> Result<(), String> {
            self.lock().unwrap().calls.push(format!("halt {id}"));
            Ok(())
        }
        fn guest_send(&mut self, _: &str, payload: &str) -> Result<(), String> {
            self.lock().unwrap().calls.push(format!("send {payload}"));
            Ok(())
        }
        fn guest_connected(&self, _: &str) -> bool {
            true
        }
        fn guest_kick(&mut self, id: &str) {
            self.lock().unwrap().calls.push(format!("kick {id}"));
        }
        fn shutdown(&mut self) {}
    }

    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    type Fixture = (
        Host<Arc<Mutex<Fake>>>,
        Arc<Mutex<Fake>>,
        Arc<Mutex<Vec<u8>>>,
        tempfile_dir::Dir,
    );

    fn host() -> Fixture {
        let fake = Arc::new(Mutex::new(Fake::default()));
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let out = Out::new(Box::new(Sink(bytes.clone())));
        (
            Host::new(fake.clone(), out),
            fake,
            bytes,
            tempfile_dir::Dir::new(),
        )
    }

    /// Minimal temp dir (no extra dev-dependency).
    mod tempfile_dir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let p = std::env::temp_dir().join(format!(
                    "pegoles-vmhost-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn params(dir: &std::path::Path) -> CreateParams {
        let disk = dir.join("disk.vhdx");
        std::fs::write(&disk, b"vhdx").unwrap();
        CreateParams {
            computer_id: ID.into(),
            disk_path: disk.to_string_lossy().into_owned(),
            efi_vars_path: String::new(),
            machine_id_path: String::new(),
            serial_log_path: dir.join("serial.log").to_string_lossy().into_owned(),
            vcpus: 2,
            memory_mb: 2048,
            seed_iso_path: None,
            display: None,
        }
    }

    fn req(id: u64, command: HostCommand) -> HostRequest {
        HostRequest { id, command }
    }

    #[test]
    fn full_lifecycle_speaks_the_macos_command_set() {
        let (mut h, fake, bytes, dir) = host();
        let p = params(&dir.0);
        assert!(
            h.handle(req(1, HostCommand::Create { params: p.clone() }))
                .ok
        );
        let r = h.handle(req(
            2,
            HostCommand::Start {
                computer_id: ID.into(),
            },
        ));
        assert_eq!((r.ok, r.state), (true, Some(VmState::Running)));
        assert_eq!(
            h.handle(req(
                3,
                HostCommand::Pause {
                    computer_id: ID.into()
                }
            ))
            .state,
            Some(VmState::Paused)
        );
        assert_eq!(
            h.handle(req(
                4,
                HostCommand::Resume {
                    computer_id: ID.into()
                }
            ))
            .state,
            Some(VmState::Running)
        );
        assert!(
            h.handle(req(
                5,
                HostCommand::GuestSend {
                    computer_id: ID.into(),
                    payload: "{}".into()
                }
            ))
            .ok
        );
        assert_eq!(
            h.handle(req(
                6,
                HostCommand::GuestStatus {
                    computer_id: ID.into()
                }
            ))
            .connected,
            Some(true)
        );
        assert_eq!(
            h.handle(req(
                7,
                HostCommand::Stop {
                    computer_id: ID.into()
                }
            ))
            .state,
            Some(VmState::Stopped)
        );
        // A second start boots a NEW compute system for the same computer.
        assert!(
            h.handle(req(
                8,
                HostCommand::Start {
                    computer_id: ID.into()
                }
            ))
            .ok
        );
        assert!(
            h.handle(req(
                9,
                HostCommand::Destroy {
                    computer_id: ID.into()
                }
            ))
            .ok
        );
        let calls = fake.lock().unwrap().calls.clone();
        assert_eq!(
            calls,
            vec![
                format!("boot {ID}"),
                format!("pause {ID}"),
                format!("resume {ID}"),
                "send {}".into(),
                format!("halt {ID}"),
                format!("boot {ID}"),
                format!("halt {ID}"),
            ]
        );
        let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(text
            .lines()
            .all(|l| l.contains("\"event\":\"vm_state_changed\"")));
        // Running, Paused, Running, Stopped, Running (destroy forgets silently).
        assert_eq!(text.lines().count(), 5);
    }

    #[test]
    fn refuses_bad_params_unknown_commands_and_unbounded_frames() {
        let (mut h, fake, _, dir) = host();
        let mut p = params(&dir.0);
        p.disk_path = p.disk_path.replace(".vhdx", ".raw");
        assert_eq!(
            h.handle(req(1, HostCommand::Create { params: p }))
                .error
                .unwrap()
                .code,
            HostErrorCode::InvalidParams
        );
        let mut p = params(&dir.0);
        p.computer_id = "..\\evil".into();
        assert!(!h.handle(req(2, HostCommand::Create { params: p })).ok);
        let r = h.handle_line(r#"{"id":9,"command":"exec","cmd":"calc.exe"}"#);
        assert_eq!((r.id, r.ok), (9, false));
        assert!(
            h.handle(req(
                3,
                HostCommand::Create {
                    params: params(&dir.0)
                }
            ))
            .ok
        );
        let big = "x".repeat(pegoles_guest_proto::MAX_FRAME_BYTES + 1);
        assert!(
            !h.handle(req(
                4,
                HostCommand::GuestSend {
                    computer_id: ID.into(),
                    payload: big
                }
            ))
            .ok
        );
        assert!(
            !h.handle(req(
                5,
                HostCommand::GuestSend {
                    computer_id: ID.into(),
                    payload: "a\nb".into()
                }
            ))
            .ok
        );
        assert!(fake.lock().unwrap().calls.is_empty());
    }

    #[test]
    fn a_failed_boot_says_why_and_stays_stopped() {
        let (mut h, fake, _, dir) = host();
        fake.lock().unwrap().fail_boot = true;
        assert!(
            h.handle(req(
                1,
                HostCommand::Create {
                    params: params(&dir.0)
                }
            ))
            .ok
        );
        let r = h.handle(req(
            2,
            HostCommand::Start {
                computer_id: ID.into(),
            },
        ));
        assert!(!r.ok);
        assert!(r.error.unwrap().message.contains("0x80370102"));
        assert_eq!(
            h.handle(req(
                3,
                HostCommand::State {
                    computer_id: ID.into()
                }
            ))
            .state,
            Some(VmState::Stopped)
        );
    }

    #[test]
    fn off_windows_every_boot_fails_closed() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let mut h = Host::new(Unavailable, Out::new(Box::new(Sink(bytes))));
        let dir = tempfile_dir::Dir::new();
        assert!(
            h.handle(req(
                1,
                HostCommand::Create {
                    params: params(&dir.0)
                }
            ))
            .ok
        );
        assert!(
            !h.handle(req(
                2,
                HostCommand::Start {
                    computer_id: ID.into()
                }
            ))
            .ok
        );
    }
}

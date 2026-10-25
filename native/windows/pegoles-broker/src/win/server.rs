//! The broker's named-pipe server: one thread per connected helper, a
//! closed set of verbs, VMs leased to the connection that made them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use pegoles_broker_proto as proto;
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use super::client::{self, Client};
use super::files;
use super::hcs;
use super::util::{local_free, pcwstr, wide, Owned};

/// Who may open the pipe: SYSTEM and administrators fully; interactive
/// users may read and write but NOT create pipe instances
/// (FILE_CREATE_PIPE_INSTANCE is left out of 0x12018b), so no user
/// process can pose as the broker to other clients.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12018b;;;IU)";
const BUFFER_BYTES: u32 = 64 * 1024;

/// Asks the accept loop to stop (it wakes itself with a connection).
#[derive(Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
        // Unblock ConnectNamedPipe with a throwaway connection.
        let name = wide(proto::PIPE_NAME);
        // SAFETY: plain open of our own pipe; closed immediately.
        if let Ok(h) = unsafe {
            CreateFileW(
                pcwstr(&name),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        } {
            drop(Owned(h));
        }
    }

    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Connected helpers right now (the service stops itself when idle).
pub static SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// Compute systems per user SID, across all sessions (quota).
fn per_user() -> &'static Mutex<HashMap<String, usize>> {
    static MAP: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptor…, freed once.
        unsafe { local_free(self.0 .0) };
    }
}

fn pipe_security() -> Result<SecurityDescriptor, String> {
    let sddl = wide(PIPE_SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: NUL-terminated SDDL; the OS allocates the descriptor.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            pcwstr(&sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| format!("pipe security: {e}"))?;
    Ok(SecurityDescriptor(sd))
}

fn create_instance(first: bool, sd: &SecurityDescriptor) -> Result<Owned, String> {
    let name = wide(proto::PIPE_NAME);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0 .0,
        bInheritHandle: false.into(),
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        // Fails if anyone already owns the name: nobody can squat it.
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: NUL-terminated name, valid security attributes.
    let pipe = unsafe {
        CreateNamedPipeW(
            pcwstr(&name),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER_BYTES,
            BUFFER_BYTES,
            0,
            Some(&attributes),
        )
    };
    if pipe.is_invalid() {
        return Err(format!(
            "cannot create {}: {}",
            proto::PIPE_NAME,
            windows::core::Error::from_win32()
        ));
    }
    Ok(Owned(pipe))
}

fn expected_client() -> Result<PathBuf, String> {
    super::util::require_admin_only_folder()?;
    let exe = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    let dir = exe.parent().ok_or("own folder")?;
    Ok(dir.join(proto::CLIENT_EXE))
}

/// Accept helpers until `stop`.
pub fn serve(stop: &StopFlag) -> Result<(), String> {
    let sd = pipe_security()?;
    let expected = expected_client()?;
    let mut first = true;
    while !stop.stopped() {
        let pipe = create_instance(first, &sd)?;
        first = false;
        // SAFETY: a fresh server pipe; blocking connect.
        let connected = unsafe { ConnectNamedPipe(pipe.raw(), None) };
        if let Err(e) = connected {
            if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                continue;
            }
        }
        if stop.stopped() {
            break;
        }
        let expected = expected.clone();
        SESSIONS.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("pegoles-broker-session".into())
            .spawn(move || {
                session(pipe, &expected);
                SESSIONS.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            SESSIONS.fetch_sub(1, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// Read one line (bounded). None on EOF, error or an oversized line.
fn read_line(pipe: HANDLE, pending: &mut Vec<u8>) -> Option<String> {
    loop {
        if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=pos).collect();
            let text = String::from_utf8(line[..line.len() - 1].to_vec()).ok()?;
            return Some(text.trim_end_matches('\r').to_string());
        }
        if pending.len() > proto::MAX_LINE_BYTES {
            return None;
        }
        let mut buffer = [0u8; 4096];
        let mut read = 0u32;
        // SAFETY: valid pipe, buffer and length from the same array.
        unsafe { ReadFile(pipe, Some(&mut buffer), Some(&mut read), None) }.ok()?;
        if read == 0 {
            return None;
        }
        pending.extend_from_slice(&buffer[..read as usize]);
    }
}

fn write_line(pipe: HANDLE, text: &str) -> bool {
    let mut written = 0u32;
    // SAFETY: valid pipe; the slice is the whole line.
    let ok = unsafe { WriteFile(pipe, Some(text.as_bytes()), Some(&mut written), None) }.is_ok();
    ok && written as usize == text.len() && unsafe { FlushFileBuffers(pipe) }.is_ok()
}

struct Vm {
    system: hcs::System,
    disk: String,
    /// Keeps the computer's folders pinned while the VM exists.
    _lease: files::DiskLease,
}

/// One helper's connection. Its VMs end when it does (the lease).
fn session(pipe: Owned, expected: &std::path::Path) {
    let client = match client::identify(pipe.raw(), expected) {
        Ok(client) => client,
        Err(e) => {
            let _ = write_line(
                pipe.raw(),
                &proto::encode_response(&proto::Response::fail("forbidden", e)),
            );
            // SAFETY: our server end; the client is dropped.
            unsafe {
                let _ = DisconnectNamedPipe(pipe.raw());
            }
            return;
        }
    };
    let mut vms: HashMap<String, Vm> = HashMap::new();
    let mut pending = Vec::new();
    while let Some(line) = read_line(pipe.raw(), &mut pending) {
        let response = match proto::parse_request(&line) {
            Ok(request) => handle(&client, &mut vms, request),
            Err(e) => proto::Response::fail("bad_request", e),
        };
        if !write_line(pipe.raw(), &proto::encode_response(&response)) {
            break;
        }
    }
    for (id, vm) in vms.drain() {
        release(&client, &id, vm);
    }
    // SAFETY: our server end.
    unsafe {
        let _ = DisconnectNamedPipe(pipe.raw());
    }
}

fn release(client: &Client, id: &str, vm: Vm) {
    let _ = hcs::terminate(&vm.system);
    drop(vm.system);
    let _ = client.as_user(|| {
        hcs::revoke_vm_access(id, &vm.disk);
        Ok(())
    });
    if let Ok(mut counts) = per_user().lock() {
        if let Some(n) = counts.get_mut(&client.sid) {
            *n = n.saturating_sub(1);
        }
    }
}

fn state_of(system: &hcs::System) -> Option<String> {
    let props = hcs::properties(system).ok()?;
    props
        .get("State")
        .and_then(|s| s.as_str())
        .and_then(proto::state_word)
        .map(str::to_string)
}

fn handle(
    client: &Client,
    vms: &mut HashMap<String, Vm>,
    request: proto::Request,
) -> proto::Response {
    use proto::Request as R;
    match request {
        R::Hello { version } => {
            if version != proto::PROTOCOL_VERSION {
                return proto::Response::fail(
                    "bad_request",
                    format!("protocol {version} is not supported"),
                );
            }
            proto::Response {
                broker_version: Some(env!("CARGO_PKG_VERSION").into()),
                ..proto::Response::ok()
            }
        }
        R::Create(spec) => create(client, vms, spec),
        R::Start { computer_id } => with_vm(vms, &computer_id, |vm| hcs::start(&vm.system)),
        R::Pause { computer_id } => with_vm(vms, &computer_id, |vm| hcs::pause(&vm.system)),
        R::Resume { computer_id } => with_vm(vms, &computer_id, |vm| hcs::resume(&vm.system)),
        R::Shutdown { computer_id } => match vms.remove(&computer_id) {
            Some(vm) => {
                // Graceful first; the lease ends either way.
                if hcs::shutdown(&vm.system).is_err() {
                    let _ = hcs::terminate(&vm.system);
                }
                release(client, &computer_id, vm);
                proto::Response {
                    state: Some("stopped".into()),
                    ..proto::Response::ok()
                }
            }
            None => proto::Response::fail("unknown_computer", "no such computer in this session"),
        },
        R::Terminate { computer_id } => match vms.remove(&computer_id) {
            Some(vm) => {
                release(client, &computer_id, vm);
                proto::Response {
                    state: Some("stopped".into()),
                    ..proto::Response::ok()
                }
            }
            None => proto::Response::fail("unknown_computer", "no such computer in this session"),
        },
        R::State { computer_id } => match vms.get(&computer_id) {
            Some(vm) => proto::Response {
                state: state_of(&vm.system),
                ..proto::Response::ok()
            },
            None => proto::Response {
                state: Some("stopped".into()),
                ..proto::Response::ok()
            },
        },
    }
}

fn with_vm(
    vms: &HashMap<String, Vm>,
    id: &str,
    op: impl FnOnce(&Vm) -> Result<(), String>,
) -> proto::Response {
    match vms.get(id) {
        Some(vm) => match op(vm) {
            Ok(()) => proto::Response {
                state: state_of(&vm.system),
                ..proto::Response::ok()
            },
            Err(e) => proto::Response::fail("hcs", e),
        },
        None => proto::Response::fail("unknown_computer", "no such computer in this session"),
    }
}

fn create(
    client: &Client,
    vms: &mut HashMap<String, Vm>,
    spec: proto::CreateSpec,
) -> proto::Response {
    if let Err(e) = spec.validate() {
        return proto::Response::fail("bad_request", e);
    }
    if vms.contains_key(&spec.computer_id) {
        return proto::Response::fail("bad_request", "this computer already exists; stop it first");
    }
    let disk = proto::computer_file(&client.local_app_data, &spec.computer_id, proto::DISK_FILE);
    if !proto::same_windows_path(&disk, &spec.disk) {
        return proto::Response::fail(
            "forbidden",
            "the disk must be the computer's own disk.vhdx in the user's Pegoles folder",
        );
    }
    let lease = match files::check_disk(client, &spec.computer_id, &disk) {
        Ok(lease) => lease,
        Err(e) => return proto::Response::fail("forbidden", e),
    };
    {
        let Ok(mut counts) = per_user().lock() else {
            return proto::Response::fail("internal", "quota lock");
        };
        let n = counts.entry(client.sid.clone()).or_insert(0);
        if *n >= proto::MAX_COMPUTERS_PER_USER {
            return proto::Response::fail(
                "limit",
                "too many Pegoles computers are running for this user",
            );
        }
        *n += 1;
    }
    let undo_count = || {
        if let Ok(mut counts) = per_user().lock() {
            if let Some(n) = counts.get_mut(&client.sid) {
                *n = n.saturating_sub(1);
            }
        }
    };
    let document = proto::hcs_document(&spec, &disk, &client.sid);
    let system = match hcs::create(&spec.computer_id, &document) {
        Ok(system) => system,
        Err(e) => {
            undo_count();
            return proto::Response::fail("hcs", e);
        }
    };
    // Grant the VM access as the user: works only on files they control.
    if let Err(e) = client.as_user(|| hcs::grant_vm_access(&spec.computer_id, &disk)) {
        let _ = hcs::terminate(&system);
        undo_count();
        return proto::Response::fail("hcs", e);
    }
    let runtime_id = hcs::properties(&system)
        .ok()
        .and_then(|p| {
            p.get("RuntimeId")
                .and_then(|r| r.as_str())
                .map(str::to_lowercase)
        })
        .filter(|r| proto::is_uuid(r));
    let Some(runtime_id) = runtime_id else {
        let _ = hcs::terminate(&system);
        client
            .as_user(|| {
                hcs::revoke_vm_access(&spec.computer_id, &disk);
                Ok(())
            })
            .ok();
        undo_count();
        return proto::Response::fail("hcs", "HCS did not report the VM's runtime id");
    };
    vms.insert(
        spec.computer_id.clone(),
        Vm {
            system,
            disk,
            _lease: lease,
        },
    );
    proto::Response {
        runtime_id: Some(runtime_id),
        state: Some("created".into()),
        ..proto::Response::ok()
    }
}

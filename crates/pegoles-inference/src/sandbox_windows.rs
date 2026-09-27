//! Windows confinement for the llama.cpp worker.
//!
//! - **AppContainer, no capabilities**: no network (no `internetClient`
//!   or any other capability), no access to the user's files, registry
//!   keys or named objects unless explicitly granted. The only grant is
//!   read/execute on the model store (and, in debug builds, the worker's
//!   build folder; installed builds live under Program Files, which
//!   AppContainers can already read).
//! - **Child-process policy**: the worker cannot start any process.
//! - **Job object**: killed when Pegoles closes the job handle (quit or
//!   crash), one active process, a committed-memory cap, no desktop /
//!   clipboard / global-atom / system-parameter access, and no Windows
//!   Error Reporting dialog on a crash.
//! - **Inheritance**: exactly the three pipe ends, through an explicit
//!   handle list.
//! - **Environment**: rebuilt from scratch (system root, a temp folder
//!   inside the AppContainer's own storage, the Vulkan loader told to skip
//!   implicit layers such as overlays).
//!
//! Anything that fails here fails closed: the model does not run.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};

use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL,
    WAIT_OBJECT_0, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP,
    TRUSTEE_W,
};
use windows::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
};
use windows::Win32::Security::{
    FreeSid, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SECURITY_CAPABILITIES, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::backend::InferenceError;
use crate::llama::{ChildProc, LlamaWorkerConfig, Spawned};

/// The worker's AppContainer (created on first use, reused afterwards).
const CONTAINER_NAME: &str = "Pegoles.ModelWorker";
const CONTAINER_DISPLAY: &str = "Pegoles model worker";
const CONTAINER_DESCRIPTION: &str =
    "Runs the local Pegoles model with no network and no access to your files.";
/// `ProcThreadAttributeValue(14, FALSE, TRUE, FALSE)`.
const PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY: usize = 0x0002_000E;
const PROCESS_CREATION_CHILD_PROCESS_RESTRICTED: u32 = 0x01;
const PIPE_BUFFER: u32 = 1 << 20;
const EXIT_WAIT_MS: u32 = 5_000;

fn fail(what: &str, e: impl std::fmt::Display) -> InferenceError {
    InferenceError::RuntimeMissing(format!("could not isolate the model worker ({what}): {e}"))
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn win32(what: &str, code: WIN32_ERROR) -> Result<(), InferenceError> {
    if code.0 == 0 {
        Ok(())
    } else {
        Err(fail(what, windows::core::Error::from(code.to_hresult())))
    }
}

/// An owned kernel handle.
struct Handle(HANDLE);

// SAFETY: kernel handles may be used and closed from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own the handle.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// The AppContainer SID, freed with `FreeSid`.
struct ContainerSid(PSID);

impl Drop for ContainerSid {
    fn drop(&mut self) {
        // SAFETY: allocated by Create/DeriveAppContainer…
        unsafe { FreeSid(self.0) };
    }
}

fn container_sid() -> Result<ContainerSid, InferenceError> {
    let name = wide(CONTAINER_NAME.as_ref());
    let display = wide(CONTAINER_DISPLAY.as_ref());
    let description = wide(CONTAINER_DESCRIPTION.as_ref());
    // SAFETY: NUL-terminated strings; no capabilities.
    let created = unsafe {
        CreateAppContainerProfile(
            PCWSTR(name.as_ptr()),
            PCWSTR(display.as_ptr()),
            PCWSTR(description.as_ptr()),
            None,
        )
    };
    match created {
        Ok(sid) => Ok(ContainerSid(sid)),
        Err(e) if e.code() == HRESULT::from_win32(ERROR_ALREADY_EXISTS.0) => {
            // SAFETY: NUL-terminated name.
            unsafe { DeriveAppContainerSidFromAppContainerName(PCWSTR(name.as_ptr())) }
                .map(ContainerSid)
                .map_err(|e| fail("container", e))
        }
        Err(e) => Err(fail("container", e)),
    }
}

fn sid_string(sid: &ContainerSid) -> Result<String, InferenceError> {
    let mut out = PWSTR::null();
    // SAFETY: valid SID; the string is LocalFree'd below.
    unsafe { ConvertSidToStringSidW(sid.0, &mut out) }.map_err(|e| fail("sid", e))?;
    // SAFETY: NUL-terminated string from the API.
    let text = unsafe { out.to_string() }.map_err(|e| fail("sid", e));
    // SAFETY: allocated by ConvertSidToStringSidW.
    unsafe { LocalFree(Some(HLOCAL(out.0.cast()))) };
    text
}

/// `%LOCALAPPDATA%\Packages\<container>\AC\Temp`: the one place the
/// worker can write.
fn container_temp(sid: &ContainerSid) -> Result<PathBuf, InferenceError> {
    let text = wide(sid_string(sid)?.as_ref());
    // SAFETY: NUL-terminated SID string; result freed with CoTaskMemFree.
    let folder = unsafe { GetAppContainerFolderPath(PCWSTR(text.as_ptr())) }
        .map_err(|e| fail("storage", e))?;
    // SAFETY: NUL-terminated path from the API.
    let path = unsafe { folder.to_string() };
    // SAFETY: allocated by GetAppContainerFolderPath.
    unsafe { CoTaskMemFree(Some(folder.0 as *const c_void)) };
    let temp = PathBuf::from(path.map_err(|e| fail("storage", e))?).join("Temp");
    std::fs::create_dir_all(&temp).map_err(|e| fail("storage", e))?;
    Ok(temp)
}

/// Read + execute for the container on `dir` and everything below it.
/// Merged into the existing DACL; granting twice is a no-op.
fn grant_read(dir: &Path, sid: &ContainerSid) -> Result<(), InferenceError> {
    let path = wide(dir.as_os_str());
    let mut old: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: out-params live across the call; descriptor LocalFree'd below.
    win32("model folder", unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old),
            None,
            &mut descriptor,
        )
    })?;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: PWSTR(sid.0 .0.cast()),
            ..Default::default()
        },
    };
    let mut merged: *mut ACL = std::ptr::null_mut();
    // SAFETY: `old` belongs to `descriptor`, still alive; `merged` LocalFree'd.
    let result = win32("model folder", unsafe {
        SetEntriesInAclW(Some(&[entry]), Some(old), &mut merged)
    })
    .and_then(|()| {
        // SAFETY: valid path and ACL.
        win32("model folder", unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(merged),
                None,
            )
        })
    });
    // SAFETY: both were allocated by the calls above (or are null).
    unsafe {
        if !merged.is_null() {
            LocalFree(Some(HLOCAL(merged.cast())));
        }
        LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

/// A pipe: (parent end, child end). The child end is inheritable, the
/// parent end is not.
fn pipe(child_reads: bool) -> Result<(Handle, Handle), InferenceError> {
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    // SAFETY: out-params live across the call.
    unsafe { CreatePipe(&mut read, &mut write, Some(&attrs), PIPE_BUFFER) }
        .map_err(|e| fail("pipes", e))?;
    let (read, write) = (Handle(read), Handle(write));
    let (parent, child) = if child_reads {
        (write, read)
    } else {
        (read, write)
    };
    // SAFETY: valid handle we own.
    unsafe {
        windows::Win32::Foundation::SetHandleInformation(
            parent.0,
            HANDLE_FLAG_INHERIT.0,
            Default::default(),
        )
    }
    .map_err(|e| fail("pipes", e))?;
    Ok((parent, child))
}

/// UTF-16 environment block: sorted `KEY=VALUE` entries, double NUL.
pub(crate) fn environment_block(vars: &[(String, String)]) -> Vec<u16> {
    let mut sorted: Vec<&(String, String)> = vars.iter().collect();
    sorted.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in sorted {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

/// `"<path>"` for a command line (paths cannot contain quotes on Windows).
fn command_line(worker: &Path) -> Result<Vec<u16>, InferenceError> {
    let text = worker
        .to_str()
        .filter(|s| !s.contains('"'))
        .ok_or_else(|| fail("path", "unusable worker path"))?;
    Ok(format!("\"{text}\"")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

fn system_root() -> String {
    std::env::var("SystemRoot")
        .ok()
        .filter(|s| Path::new(s).is_absolute())
        .unwrap_or_else(|| r"C:\Windows".into())
}

fn job(memory_limit: u64) -> Result<Handle, InferenceError> {
    // SAFETY: anonymous job.
    let job =
        Handle(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|e| fail("job", e))?);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = usize::try_from(memory_limit).unwrap_or(usize::MAX);
    // SAFETY: the struct and its size match the information class.
    unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|e| fail("job limits", e))?;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    // SAFETY: as above.
    unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    }
    .map_err(|e| fail("job limits", e))?;
    Ok(job)
}

/// A process attribute list with the container, the child-process
/// policy and the inherited-handle list.
struct Attributes {
    buf: Vec<u8>,
}

impl Attributes {
    fn new(count: u32) -> Result<Self, InferenceError> {
        let mut size = 0usize;
        // SAFETY: size query; expected to "fail" with the needed size.
        let _ = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut size) };
        let mut buf = vec![0u8; size];
        // SAFETY: buffer of the size the API asked for.
        unsafe {
            InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr().cast())),
                count,
                None,
                &mut size,
            )
        }
        .map_err(|e| fail("attributes", e))?;
        Ok(Self { buf })
    }

    fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.buf.as_mut_ptr().cast())
    }

    /// SAFETY: `value` must stay alive and unmoved until the process is created.
    unsafe fn set<T>(
        &mut self,
        attribute: usize,
        value: *const T,
        size: usize,
    ) -> Result<(), InferenceError> {
        let list = self.list();
        unsafe {
            UpdateProcThreadAttribute(list, 0, attribute, Some(value.cast()), size, None, None)
        }
        .map_err(|e| fail("attributes", e))
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        let list = self.list();
        // SAFETY: initialized in new().
        unsafe { DeleteProcThreadAttributeList(list) };
    }
}

struct WinChild {
    process: Handle,
    job: Handle,
    pid: u32,
}

impl ChildProc for WinChild {
    fn pid(&self) -> Option<i32> {
        i32::try_from(self.pid).ok()
    }

    fn exited(&mut self) -> bool {
        // SAFETY: valid process handle.
        (unsafe { WaitForSingleObject(self.process.0, 0) }) == WAIT_OBJECT_0
    }

    fn terminate(&mut self) -> Option<i32> {
        // SAFETY: valid handles; terminating an already-empty job is fine.
        unsafe {
            let _ = TerminateJobObject(self.job.0, 1);
            WaitForSingleObject(self.process.0, EXIT_WAIT_MS);
        }
        let mut code = 0u32;
        // SAFETY: valid process handle.
        unsafe { GetExitCodeProcess(self.process.0, &mut code) }.ok()?;
        Some(code as i32)
    }
}

pub(crate) fn spawn(cfg: &LlamaWorkerConfig) -> Result<Spawned, InferenceError> {
    if !cfg.sandbox {
        return spawn_plain(cfg);
    }
    let worker = cfg.worker.canonicalize().map_err(|e| fail("worker", e))?;
    let worker_dir = worker
        .parent()
        .ok_or_else(|| fail("worker", "no folder"))?
        .to_path_buf();
    let sid = container_sid()?;
    grant_read(&cfg.models_root, &sid)?;
    if cfg!(debug_assertions) {
        // Installed builds run from Program Files (readable by every
        // AppContainer); a development build runs from the repository.
        grant_read(&worker_dir, &sid)?;
    }
    let temp = container_temp(&sid)?;
    let temp = temp
        .to_str()
        .ok_or_else(|| fail("storage", "path is not UTF-16 clean"))?
        .to_string();
    let root = system_root();
    let mut vars = vec![
        ("SystemRoot".to_string(), root.clone()),
        ("windir".to_string(), root.clone()),
        ("PATH".to_string(), format!(r"{root}\System32")),
        ("TEMP".to_string(), temp.clone()),
        ("TMP".to_string(), temp),
        // Overlays and capture tools register implicit Vulkan layers
        // that would otherwise load into the worker.
        (
            "VK_LOADER_LAYERS_DISABLE".to_string(),
            "~implicit~".to_string(),
        ),
    ];
    // Windows rewrites the profile folders of an AppContainer's
    // environment to the container's own storage and fails process
    // creation (ERROR_ENVVAR_NOT_FOUND) when they are absent: pass the
    // names through (paths only, no secrets) for it to rewrite.
    for name in ["USERPROFILE", "LOCALAPPDATA", "APPDATA", "SystemDrive"] {
        if let Ok(value) = std::env::var(name) {
            vars.push((name.to_string(), value));
        }
    }
    if cfg.cpu_only {
        vars.push(("PEGOLES_LLM_CPU".to_string(), "1".to_string()));
    }
    let env = environment_block(&vars);
    let mut cmd = command_line(&worker)?;
    let dir = wide(worker_dir.as_os_str());

    let (stdin_parent, stdin_child) = pipe(true)?;
    let (stdout_parent, stdout_child) = pipe(false)?;
    let (stderr_parent, stderr_child) = pipe(false)?;
    let inherit = [stdin_child.0, stdout_child.0, stderr_child.0];
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        ..Default::default()
    };
    let child_policy = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    let mut attrs = Attributes::new(3)?;
    // SAFETY: `caps`, `child_policy` and `inherit` outlive CreateProcessW below.
    unsafe {
        attrs.set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &caps,
            std::mem::size_of_val(&caps),
        )?;
        attrs.set(
            PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
            &child_policy,
            std::mem::size_of_val(&child_policy),
        )?;
        attrs.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherit.as_ptr(),
            std::mem::size_of_val(&inherit),
        )?;
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_child.0;
    startup.StartupInfo.hStdOutput = stdout_child.0;
    startup.StartupInfo.hStdError = stderr_child.0;
    startup.lpAttributeList = attrs.list();
    let job = job(cfg.memory_limit_bytes)?;
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer refers to a live local; the process starts
    // suspended so it is in the job before it runs a single instruction.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(cmd.as_mut_ptr())),
            None,
            None,
            true,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_NO_WINDOW
                | CREATE_SUSPENDED,
            Some(env.as_ptr().cast()),
            PCWSTR(dir.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .map_err(|e| fail("start", e))?;
    drop(attrs);
    let process = Handle(info.hProcess);
    let thread = Handle(info.hThread);
    // SAFETY: valid job and process handles.
    if let Err(e) = unsafe { AssignProcessToJobObject(job.0, process.0) } {
        // SAFETY: never resumed; kill it.
        let _ = unsafe { windows::Win32::System::Threading::TerminateProcess(process.0, 1) };
        return Err(fail("job", e));
    }
    // SAFETY: the suspended primary thread.
    if (unsafe { ResumeThread(thread.0) }) == u32::MAX {
        let _ = unsafe { TerminateJobObject(job.0, 1) };
        return Err(fail("start", std::io::Error::last_os_error()));
    }
    drop(thread);
    // The child ends now belong to the worker only.
    drop((stdin_child, stdout_child, stderr_child));
    let file = |h: Handle| {
        let raw = h.0 .0 as RawHandle;
        std::mem::forget(h);
        // SAFETY: we own the handle and hand it to File, which closes it.
        unsafe { std::fs::File::from_raw_handle(raw) }
    };
    Ok(Spawned {
        child: Box::new(WinChild {
            process,
            job,
            pid: info.dwProcessId,
        }),
        stdin: Box::new(file(stdin_parent)),
        stdout: Box::new(file(stdout_parent)),
        stderr: Box::new(file(stderr_parent)),
    })
}

/// Unconfined (supervisor tests only; production configs keep `sandbox`).
fn spawn_plain(cfg: &LlamaWorkerConfig) -> Result<Spawned, InferenceError> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&cfg.worker);
    cmd.env_clear()
        .env("SystemRoot", system_root())
        .creation_flags(CREATE_NO_WINDOW.0);
    if cfg.cpu_only {
        cmd.env("PEGOLES_LLM_CPU", "1");
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| InferenceError::RuntimeMissing(format!("cannot start worker: {e}")))?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    Ok(Spawned {
        child: Box::new(PlainChild(child)),
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stderr: Box::new(stderr),
    })
}

struct PlainChild(std::process::Child);

impl ChildProc for PlainChild {
    fn pid(&self) -> Option<i32> {
        i32::try_from(self.0.id()).ok()
    }

    fn exited(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(None))
    }

    fn terminate(&mut self) -> Option<i32> {
        let _ = self.0.kill();
        self.0.wait().ok().and_then(|s| s.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// `examples/stand_in_worker.rs`, built by `cargo test` next to this
    /// test binary's folder.
    fn stand_in() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let debug = exe.parent().and_then(|deps| deps.parent()).unwrap();
        let path = debug.join("examples").join("stand_in_worker.exe");
        assert!(
            path.is_file(),
            "build it first: cargo test builds examples ({})",
            path.display()
        );
        path
    }

    fn ask(
        spawned: &mut Spawned,
        reader: &mut impl BufRead,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let _ = writeln!(spawned.stdin, "{request}");
        let _ = spawned.stdin.flush();
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        match serde_json::from_str(&line) {
            Ok(reply) => reply,
            Err(_) => {
                // The stand-in died: say how.
                let code = spawned.child.terminate();
                let mut stderr = String::new();
                let _ = std::io::Read::read_to_string(&mut spawned.stderr, &mut stderr);
                panic!(
                    "no reply to {request}: exit {code:?} (0x{:08x}), stderr {stderr:?}",
                    code.unwrap_or(0) as u32
                );
            }
        }
    }

    /// Escape probes, the Windows counterpart of the macOS worker's
    /// `sandbox_blocks_escapes`: unconfined, the stand-in can read a file
    /// of the user's, reach a loopback listener and start a process;
    /// confined (AppContainer + job object), it can do none of them.
    #[test]
    fn a_confined_worker_cannot_read_files_reach_the_network_or_start_processes() {
        let secret_dir = tempfile::tempdir().unwrap();
        let secret = secret_dir.path().join("secret.txt");
        std::fs::write(&secret, b"not for the model").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || for _ in listener.incoming() {});
        let models = tempfile::tempdir().unwrap();
        let probe =
            serde_json::json!({"v": 1, "id": 2, "op": "probe", "file": secret, "addr": addr});

        for confined in [false, true] {
            eprintln!("stand-in, confined: {confined}");
            let mut cfg = LlamaWorkerConfig::new(stand_in(), models.path().to_path_buf());
            cfg.sandbox = confined;
            let mut spawned = spawn(&cfg).expect("stand-in starts");
            let stdout = std::mem::replace(&mut spawned.stdout, Box::new(std::io::empty()));
            let mut reader = BufReader::new(stdout);
            let hello = ask(
                &mut spawned,
                &mut reader,
                serde_json::json!({"v": 1, "id": 1, "op": "hello"}),
            );
            assert_eq!(hello["worker"], "pegoles-llama");
            let got = ask(&mut spawned, &mut reader, probe.clone());
            for escape in ["file_read", "network", "process"] {
                assert_eq!(
                    got[escape], !confined,
                    "{escape} (confined: {confined}): {got}"
                );
            }
            spawned.child.terminate();
        }
    }

    #[test]
    fn environment_block_is_sorted_and_double_terminated() {
        let block = environment_block(&[
            ("TMP".into(), "x".into()),
            ("SystemRoot".into(), r"C:\Windows".into()),
        ]);
        let text = String::from_utf16(&block).unwrap();
        assert_eq!(text, "SystemRoot=C:\\Windows\0TMP=x\0\0");
    }
}

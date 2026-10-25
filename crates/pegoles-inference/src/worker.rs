//! MLX backend: a persistent, supervised Python worker process.
//!
//! ```text
//! MlxWorkerBackend ──stdin JSONL (v1)──▶ pegoles_mlx_worker.py ─▶ mlx-vlm ─▶ Metal
//!                  ◀─stdout JSONL──────
//! ```
//!
//! The model stays loaded across requests. Every request is bounded
//! (request size, reply size, timeout); cancellation is cooperative first
//! (the worker stops between tokens) and forceful after a grace period
//! (kill). A crashed or killed worker is respawned on the next request
//! and the model re-verified and reloaded. The worker runs with a cleared
//! environment (no API keys, offline flags), under `sandbox-exec` with the
//! network denied and user files read-only when available. Its pipes are
//! served by threads (stdin writer, stdout reader, stderr drain), so a
//! worker that stops reading or floods stderr cannot stall or exhaust the
//! host. It runs in its own process group, which is killed as a whole.

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

use crate::backend::{
    BackendInfo, BackendMemory, GenerateRequest, GenerateResponse, InferenceBackend,
    InferenceError, LoadReport, Timings,
};
use crate::hardware;
use crate::store::{ModelStore, VerifiedModel};
pub use crate::supervisor::PROTOCOL_VERSION;
use crate::supervisor::{drain_stderr, read_replies, wire_messages, write_requests, Line};

const POLL: Duration = Duration::from_millis(25);
/// A worker whose stdout closed gets this long to exit on its own (so its
/// status can be reported) before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(1);
/// How long the stderr drain may take to catch up once the worker is gone.
const STDERR_SETTLE: Duration = Duration::from_millis(250);
/// Deepest level below the worker's temp dir that is emptied on spawn
/// (bounds the host's stack and open descriptors). Only a hostile worker
/// nests deeper; what lies below is left in place.
const MAX_CLEAR_DEPTH: usize = 16;
/// Listing passes per directory while emptying it (a directory changed
/// while listed can skip entries; something that keeps writing to it
/// cannot hold a spawn).
const MAX_CLEAR_PASSES: usize = 4;
/// Whole-cleanup budget: entries visited and wall time. A spawn never waits
/// longer than this on what an earlier (possibly hostile) worker left
/// behind; whatever is left over is tried again at the next spawn.
const MAX_CLEAR_ENTRIES: usize = 100_000;
const MAX_CLEAR_TIME: Duration = Duration::from_secs(2);

struct ClearBudget {
    entries_left: usize,
    until: Instant,
}

impl ClearBudget {
    fn new() -> Self {
        Self {
            entries_left: MAX_CLEAR_ENTRIES,
            until: Instant::now() + MAX_CLEAR_TIME,
        }
    }

    fn spend(&mut self) -> bool {
        if self.entries_left == 0 || Instant::now() >= self.until {
            self.entries_left = 0;
            return false;
        }
        self.entries_left -= 1;
        true
    }
}

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// MLX buffer-cache ceiling for the worker.
pub const DEFAULT_CACHE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// Seatbelt profile for the worker (later rules win). The worker is
/// treated as compromisable (it parses model files and runs a large native
/// stack), so the profile removes every way out that it does not need:
/// - no network at all (TCP, UDP, DNS and unix sockets);
/// - no fork (nothing can leave the process group the supervisor kills);
/// - no exec of anything but its own interpreter (so no `open`, no
///   `osascript`, no shell), no Apple Events;
/// - no Mach services except the Metal compiler: in particular no
///   LaunchServices (which would launch an unsandboxed app or open a
///   URL for it), pasteboard, Keychain (securityd), WindowServer or
///   preferences daemons;
/// - no signals to, and no inspection of (arguments, environment),
///   other processes; IOKit limited to the GPU and IOSurface clients;
/// - no file contents under the user's home except the runtime, the model
///   store and the worker script (metadata stays readable: Python
///   resolves its own path);
/// - writes only to a private temporary directory.
///
/// `tests::sandbox_blocks_escapes` pins these against the real runtime.
const SANDBOX_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-fork)
(deny process-exec*)
(allow process-exec (literal (param "PYTHON")))
(deny appleevent-send)
(deny signal)
(allow signal (target self))
(deny process-info*)
(allow process-info* (target self))
(deny iokit-open)
(allow iokit-open (iokit-user-client-class "AGXDeviceUserClient" "IOSurfaceRootUserClient"))
(deny mach-lookup)
(allow mach-lookup (global-name "com.apple.MTLCompilerService"))
(deny file-read-data (subpath (param "HOME")))
(allow file-read-data (literal (param "HOME")) (subpath (param "RUNTIME")) (subpath (param "MODELS")) (subpath (param "SCRIPT_DIR")))
(deny file-write*)
(allow file-write* (subpath (param "TMP")) (literal "/dev/null") (literal "/dev/dtracehelper"))
"#;

#[derive(Clone, Debug)]
pub struct MlxWorkerConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    /// The model store root (the only model location the worker can read).
    pub models_root: PathBuf,
    /// Run under `sandbox-exec`. Mandatory in production: if it is
    /// unavailable the worker does not start (fail closed). Tests of the
    /// supervisor with a stand-in shell script turn it off.
    pub sandbox: bool,
    /// MLX buffer-cache ceiling (freed buffers above it go back to the
    /// system). `None` keeps MLX's default (unbounded up to its own
    /// heuristics).
    pub cache_limit_bytes: Option<u64>,
    pub hello_timeout: Duration,
    pub load_timeout: Duration,
    pub cancel_grace: Duration,
}

impl MlxWorkerConfig {
    pub fn new(python: PathBuf, script: PathBuf, models_root: PathBuf) -> Self {
        Self {
            python,
            script,
            models_root,
            sandbox: true,
            // Measured (worker_probe, MAI-UI 2B 6-bit): peak footprint
            // 4.9 -> 4.0 GB, same latency, same steady state.
            cache_limit_bytes: Some(DEFAULT_CACHE_LIMIT_BYTES),
            hello_timeout: Duration::from_secs(60),
            load_timeout: Duration::from_secs(240),
            cancel_grace: Duration::from_secs(3),
        }
    }

    /// The product layout: the interpreter shipped inside the signed app
    /// bundle (`Resources/runtime/python`, built by
    /// `scripts/local-model/build-runtime.sh`) and the worker script next
    /// to it (`Resources/workers/mlx/`). Debug builds also accept the
    /// repository's build output. Release builds never run a Python found
    /// in a user-writable location such as the data directory.
    pub fn discover(data_dir: &Path) -> Result<Self, InferenceError> {
        Self::discover_from(&runtime_candidates(), &worker_script_candidates(), data_dir)
    }

    fn discover_from(
        pythons: &[PathBuf],
        scripts: &[PathBuf],
        data_dir: &Path,
    ) -> Result<Self, InferenceError> {
        let python = pythons.iter().find(|p| p.is_file()).ok_or_else(|| {
            InferenceError::RuntimeMissing(
                "the Pegoles Local runtime is missing from this installation; reinstall Pegoles"
                    .into(),
            )
        })?;
        let script = scripts.iter().find(|p| p.is_file()).ok_or_else(|| {
            InferenceError::RuntimeMissing("the local model worker is missing".into())
        })?;
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(InferenceError::RuntimeMissing(
                "macOS sandboxing (sandbox-exec) is unavailable, so Pegoles Local cannot run \
                 its model safely on this Mac"
                    .into(),
            ));
        }
        let canonical = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
        Ok(Self::new(
            canonical(python),
            canonical(script),
            crate::models_dir(data_dir),
        ))
    }

    /// `sandbox-exec` arguments: the profile and its parameters. Values are
    /// passed as `-D` parameters, never spliced into the profile text.
    fn sandbox_args(&self, tmp: &Path) -> Result<Vec<String>, InferenceError> {
        let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let param = |p: &Path| -> Result<String, InferenceError> {
            p.to_str()
                .filter(|s| p.is_absolute() && !s.contains('"'))
                .map(str::to_string)
                .ok_or_else(|| {
                    InferenceError::RuntimeMissing(format!("unusable path {}", p.display()))
                })
        };
        let abs = |p: &Path| param(&canonical(p));
        // Only the temp dir's parent is resolved: the worker can replace
        // the temp dir itself with a symlink, which must not turn into
        // write access to the link's target for the next worker.
        let tmp = match (tmp.parent(), tmp.file_name()) {
            (Some(parent), Some(name)) => param(&canonical(parent).join(name))?,
            _ => return Err(InferenceError::RuntimeMissing("bad worker temp dir".into())),
        };
        let home = std::env::var("HOME").map_err(|_| {
            InferenceError::RuntimeMissing("HOME is not set; cannot sandbox the worker".into())
        })?;
        // bin/python3.12 -> the runtime root.
        let venv = self
            .python
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| InferenceError::RuntimeMissing("bad runtime path".into()))?;
        let script_dir = self
            .script
            .parent()
            .ok_or_else(|| InferenceError::RuntimeMissing("bad worker path".into()))?;
        Ok(vec![
            "-D".into(),
            format!("HOME={}", abs(Path::new(&home))?),
            "-D".into(),
            format!("TMP={tmp}"),
            "-D".into(),
            format!("PYTHON={}", abs(&self.python)?),
            "-D".into(),
            format!("RUNTIME={}", abs(venv)?),
            "-D".into(),
            format!("MODELS={}", abs(&self.models_root)?),
            "-D".into(),
            format!("SCRIPT_DIR={}", abs(script_dir)?),
            "-p".into(),
            SANDBOX_PROFILE.into(),
        ])
    }
}

/// The worker's private temporary directory: `pegoles-mlx` inside the
/// per-user Darwin temp dir. It is the only place the sandboxed worker may
/// write. Returned with a descriptor on it (see `open_private_dir`).
fn worker_tmp_dir() -> Result<(PathBuf, OwnedFd), InferenceError> {
    named_tmp_dir("pegoles-mlx")
}

fn named_tmp_dir(name: &str) -> Result<(PathBuf, OwnedFd), InferenceError> {
    let base = darwin_user_temp_dir()
        .ok_or_else(|| InferenceError::RuntimeMissing("no per-user temporary directory".into()))?;
    let dir = base.join(name);
    let fd = open_private_dir(&dir)?;
    Ok((dir, fd))
}

/// A fresh, emptied private temp dir for another sandboxed worker (the
/// llama.cpp one), with the same guarantees as the MLX worker's.
#[cfg(target_os = "macos")]
pub(crate) fn fresh_private_tmp(name: &str) -> Result<PathBuf, InferenceError> {
    let (dir, fd) = named_tmp_dir(name)?;
    clear_dir(fd, 0, &mut ClearBudget::new());
    Ok(dir)
}

/// Creates `dir` (mode 0700) if needed and opens it: it must be a real
/// directory owned by this user, never a symlink, and is made private
/// (0700) through the descriptor, never through the path, which the
/// worker can swap (it may write to the directory itself).
fn open_private_dir(dir: &Path) -> Result<OwnedFd, InferenceError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let fail = |e: std::io::Error| InferenceError::RuntimeMissing(format!("worker temp dir: {e}"));
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(fail(e)),
    }
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| InferenceError::RuntimeMissing("bad worker temp dir".into()))?;
    let dir = std::fs::File::from(open_dir_at(libc::AT_FDCWD, &path).map_err(fail)?);
    let meta = dir.metadata().map_err(fail)?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid {
        return Err(InferenceError::RuntimeMissing(
            "the worker temp dir is not a directory owned by this user".into(),
        ));
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        dir.set_permissions(std::fs::Permissions::from_mode(0o700))
            .map_err(fail)?;
    }
    Ok(dir.into())
}

/// Opens `name` in `parent` as a directory: never through a symlink and
/// never blocking (a FIFO is refused, not opened). A directory made
/// unreadable is first given back to its owner (not through a symlink).
fn open_dir_at(parent: RawFd, name: &CStr) -> std::io::Result<OwnedFd> {
    let open = || {
        let flags = libc::O_RDONLY
            | libc::O_DIRECTORY
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC;
        // SAFETY: `parent` is an open directory (or AT_FDCWD) and `name` is
        // NUL-terminated.
        let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `fd` was just opened and nothing else owns it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    match open() {
        Err(e) if e.raw_os_error() == Some(libc::EACCES) => {
            // SAFETY: as above; AT_SYMLINK_NOFOLLOW never reaches a link's target.
            unsafe { libc::fchmodat(parent, name.as_ptr(), 0o700, libc::AT_SYMLINK_NOFOLLOW) };
            open()
        }
        r => r,
    }
}

/// Removes everything inside the directory open as `dir` (not the
/// directory itself), best effort and within `budget`. Entries a worker
/// made immutable (chflags) or nested deeper than `MAX_CLEAR_DEPTH` can
/// survive; nothing on the host reads this directory. It works through descriptors only: each
/// subdirectory is opened relative to its parent's descriptor without
/// following symlinks, and each removal is an `unlinkat` of one name in an
/// open directory. Whatever the worker (or something it forked) swaps for
/// a symlink, at any level and at any moment, a removal may fail but never
/// reaches outside the directory that was opened.
fn clear_dir(dir: OwnedFd, depth: usize, budget: &mut ClearBudget) {
    // SAFETY: `dir` is an open directory descriptor.
    let stream = unsafe { libc::fdopendir(dir.as_raw_fd()) };
    if stream.is_null() {
        return;
    }
    // The stream owns the descriptor now (closedir closes it).
    let fd = dir.into_raw_fd();
    for _ in 0..MAX_CLEAR_PASSES {
        let mut removed = false;
        loop {
            // SAFETY: `stream` is open; the entry is copied before the next call.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            // SAFETY: `d_name` of a returned entry is NUL-terminated.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_owned();
            if !matches!(name.as_bytes(), b"." | b"..") {
                if !budget.spend() {
                    break;
                }
                removed |= remove_entry(fd, &name, depth, budget);
            }
        }
        if !removed || budget.entries_left == 0 {
            break;
        }
        // SAFETY: `stream` is open.
        unsafe { libc::rewinddir(stream) };
    }
    // SAFETY: `stream` is open and closed exactly once.
    unsafe { libc::closedir(stream) };
}

/// Removes one entry of the open directory `parent` (at `depth`): a real
/// directory is emptied (above `MAX_CLEAR_DEPTH`) and removed; anything
/// else, a symlink included, is unlinked where it is, never followed.
fn remove_entry(parent: RawFd, name: &CStr, depth: usize, budget: &mut ClearBudget) -> bool {
    let flags = match open_dir_at(parent, name) {
        Ok(sub) => {
            if depth < MAX_CLEAR_DEPTH {
                let sub = std::fs::File::from(sub);
                // Listing and unlinking inside it need rwx.
                let _ = sub.set_permissions(std::fs::Permissions::from_mode(0o700));
                clear_dir(sub.into(), depth + 1, budget);
            }
            libc::AT_REMOVEDIR
        }
        Err(_) => 0,
    };
    // SAFETY: `parent` is an open directory and `name` is NUL-terminated.
    unsafe { libc::unlinkat(parent, name.as_ptr(), flags) == 0 }
}

#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: the buffer is valid for `buf.len()` bytes; confstr writes at
    // most that many bytes including the terminating NUL.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if n == 0 || n > buf.len() {
        return None;
    }
    buf.truncate(n - 1);
    let p = PathBuf::from(std::ffi::OsString::from_vec(buf));
    p.is_absolute().then_some(p)
}

#[cfg(not(target_os = "macos"))]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    None
}

/// Interpreter path inside a runtime tree built by `build-runtime.sh`.
const RUNTIME_PYTHON: &str = "runtime/python/bin/python3.12";

fn runtime_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources").join(RUNTIME_PYTHON));
        }
    }
    if cfg!(debug_assertions) {
        out.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/pegoles-runtime/python/bin/python3.12"),
        );
    }
    out
}

fn worker_script_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources/workers/mlx/pegoles_mlx_worker.py"));
        }
    }
    if cfg!(debug_assertions) {
        out.push(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workers/mlx/pegoles_mlx_worker.py"),
        );
    }
    out
}

struct Proc {
    child: Child,
    /// Request lines, written in order by a dedicated thread: a worker that
    /// stops reading stdin stalls that thread, never `request()`, whose
    /// timeout and cancel rules then kill the worker.
    stdin: Sender<Vec<u8>>,
    rx: Receiver<Line>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// Disconnects once the stderr drain has read everything.
    stderr_done: Receiver<()>,
    /// Also the id of the worker's process group.
    pid: i32,
    /// Set once the worker has been waited for (its group id is then free).
    reaped: bool,
}

/// A worker never outlives its `Proc`: dropping it on any path (a failed
/// spawn included) kills its process group and reaps it.
impl Drop for Proc {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl Proc {
    fn kill(self) {
        drop(self);
    }

    /// Whether the worker has exited, without reaping it: until `terminate`
    /// reaps it, its pid keeps its process group id from being reused.
    fn exited(&self) -> bool {
        // SAFETY: an all-zero siginfo_t is a valid value.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        // SAFETY: `info` is a valid out-pointer; WNOWAIT leaves the child unreaped.
        let r = unsafe { libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, flags) };
        r != 0 || si_pid(&info) != 0
    }

    /// Kills the worker's whole process group (the worker and whatever it
    /// forked; a process that left the group with `setsid` is out of
    /// reach, still sandboxed) and reaps the worker. The group is signalled
    /// before the reap, never after, so the signal cannot reach a group
    /// that reuses the id.
    fn terminate(&mut self) -> Option<ExitStatus> {
        if !self.reaped {
            // SAFETY: killpg has no memory-safety preconditions.
            unsafe { libc::killpg(self.pid, libc::SIGKILL) };
            let _ = self.child.kill();
            self.reaped = true;
        }
        self.child.wait().ok()
    }

    /// Lets the worker exit on its own within `grace` (for its status),
    /// then kills its group: never an unbounded wait.
    fn reap(&mut self, grace: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + grace;
        while !self.exited() && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        self.terminate()
    }

    fn stderr_tail(&self, n: usize) -> String {
        let lines = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let skip = lines.len().saturating_sub(n);
        lines
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

#[cfg(target_vendor = "apple")]
fn si_pid(info: &libc::siginfo_t) -> libc::pid_t {
    info.si_pid
}

#[cfg(not(target_vendor = "apple"))]
fn si_pid(info: &libc::siginfo_t) -> libc::pid_t {
    // SAFETY: `info` was filled by waitid (or zeroed), so the field is set.
    unsafe { info.si_pid() }
}

#[derive(Clone, Debug, Default)]
pub struct WorkerHello {
    pub worker_version: String,
    pub mlx: String,
    pub mlx_vlm: String,
    pub metal: bool,
}

pub struct MlxWorkerBackend {
    cfg: MlxWorkerConfig,
    proc: Option<Proc>,
    next_id: u64,
    loaded: Option<VerifiedModel>,
    hello: Option<WorkerHello>,
    /// Restarts since creation (crash recovery count).
    pub restarts: u32,
}

impl MlxWorkerBackend {
    pub fn new(cfg: MlxWorkerConfig) -> Self {
        Self {
            cfg,
            proc: None,
            next_id: 1,
            loaded: None,
            hello: None,
            restarts: 0,
        }
    }

    pub fn pid(&self) -> Option<i32> {
        self.proc.as_ref().map(|p| p.pid)
    }

    pub fn hello_info(&self) -> Option<&WorkerHello> {
        self.hello.as_ref()
    }

    pub fn loaded_model(&self) -> Option<&str> {
        self.loaded.as_ref().map(|m| m.spec.id.as_str())
    }

    fn spawn(&mut self) -> Result<(), InferenceError> {
        if !self.cfg.python.exists() {
            return Err(InferenceError::RuntimeMissing(format!(
                "python runtime not found at {}",
                self.cfg.python.display()
            )));
        }
        let mut private_tmp = None;
        let mut cmd = if self.cfg.sandbox {
            if !Path::new(SANDBOX_EXEC).exists() {
                return Err(InferenceError::RuntimeMissing(
                    "sandbox-exec is unavailable; refusing to run the model unsandboxed".into(),
                ));
            }
            let (tmp, fd) = worker_tmp_dir()?;
            clear_dir(fd, 0, &mut ClearBudget::new());
            let mut c = Command::new(SANDBOX_EXEC);
            c.args(self.cfg.sandbox_args(&tmp)?).arg(&self.cfg.python);
            private_tmp = Some(tmp);
            c
        } else {
            Command::new(&self.cfg.python)
        };
        cmd.arg("-I") // isolated: no user site, no PYTHON* env, no cwd on path
            .arg(&self.cfg.script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_DATASETS_OFFLINE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONUNBUFFERED", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .current_dir("/")
            // Its own group: killing the group also ends what it forked,
            // which would otherwise keep its pipes (and our threads) alive.
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Set after env_clear (which also drops earlier explicit values).
        match private_tmp {
            Some(tmp) => {
                cmd.env("TMPDIR", tmp);
            }
            None => {
                if let Ok(tmp) = std::env::var("TMPDIR") {
                    cmd.env("TMPDIR", tmp);
                }
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| InferenceError::RuntimeMissing(format!("cannot start worker: {e}")))?;
        let pid = child.id() as i32;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        let (lines_tx, lines_rx) = mpsc::channel();
        let (done_tx, stderr_done) = mpsc::channel::<()>();
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let sink = stderr.clone();
        // Owns the child from here: if a thread below cannot start, the
        // early return drops it, which kills and reaps the worker.
        let proc = Proc {
            child,
            stdin: lines_tx,
            rx,
            stderr,
            stderr_done,
            pid,
            reaped: false,
        };
        let thread_failed = |e: std::io::Error| InferenceError::WorkerCrashed(e.to_string());
        std::thread::Builder::new()
            .name("pegoles-mlx-stdin".into())
            .spawn(move || write_requests(stdin, lines_rx))
            .map_err(thread_failed)?;
        std::thread::Builder::new()
            .name("pegoles-mlx-stdout".into())
            .spawn(move || read_replies(stdout, tx))
            .map_err(thread_failed)?;
        std::thread::Builder::new()
            .name("pegoles-mlx-stderr".into())
            .spawn(move || {
                drain_stderr(stderr_pipe, &sink);
                drop(done_tx);
            })
            .map_err(thread_failed)?;
        self.proc = Some(proc);
        // Request ids are per process: a respawned worker starts over.
        self.next_id = 1;
        self.loaded = None;
        let reply = self.request(json!({"op": "hello"}), self.cfg.hello_timeout, &|| false)?;
        if reply.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
            self.kill();
            return Err(InferenceError::Protocol(
                "worker speaks a different protocol version".into(),
            ));
        }
        let s = |k: &str| {
            reply
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .chars()
                .take(40)
                .collect::<String>()
        };
        self.hello = Some(WorkerHello {
            worker_version: s("worker_version"),
            mlx: s("mlx"),
            mlx_vlm: s("mlx_vlm"),
            metal: reply.get("metal").and_then(Value::as_bool) == Some(true),
        });
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(p) = self.proc.take() {
            p.kill();
        }
        self.loaded = None;
    }

    /// One request/response exchange. Kills the worker on timeout, on a
    /// cancel the worker does not honor within the grace period, and on
    /// any protocol violation.
    fn request(
        &mut self,
        mut body: Value,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value, InferenceError> {
        let id = self.next_id;
        self.next_id += 1;
        body["v"] = json!(PROTOCOL_VERSION);
        body["id"] = json!(id);
        let mut line =
            serde_json::to_vec(&body).map_err(|e| InferenceError::BadRequest(e.to_string()))?;
        line.push(b'\n');
        let proc = self
            .proc
            .as_mut()
            .ok_or_else(|| InferenceError::WorkerCrashed("worker is not running".into()))?;
        if proc.stdin.send(line).is_err() {
            // The writer stopped at a failed write: stdin is closed.
            let tail = proc.stderr_tail(4);
            self.kill();
            return Err(InferenceError::WorkerCrashed(format!(
                "write failed: the worker closed its input; {tail}"
            )));
        }
        let started = Instant::now();
        let mut cancel_deadline: Option<Instant> = None;
        loop {
            let proc = self.proc.as_mut().expect("present during request");
            match proc.rx.recv_timeout(POLL) {
                Ok(Line::Reply(v)) => {
                    let rid = v.get("id").and_then(Value::as_u64);
                    if rid != Some(id) {
                        // Replies to cancel requests, or stale ones.
                        continue;
                    }
                    if v.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(v);
                    }
                    let kind = v
                        .pointer("/error/kind")
                        .and_then(Value::as_str)
                        .unwrap_or("internal");
                    let msg: String = v
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .take(300)
                        .collect();
                    return Err(match kind {
                        "bad_request" => InferenceError::BadRequest(msg),
                        "out_of_memory" => InferenceError::OutOfMemory(msg),
                        "runtime_missing" => InferenceError::RuntimeMissing(msg),
                        _ => InferenceError::Protocol(msg),
                    });
                }
                Ok(Line::Oversized) | Ok(Line::Invalid) => {
                    self.kill();
                    return Err(InferenceError::Protocol(
                        "worker sent an invalid or oversized reply".into(),
                    ));
                }
                Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    // stdout closed: the worker is exiting, or it broke the
                    // protocol and stays alive. Reaped within a bound either
                    // way (the caller holds the shared backend lock).
                    let mut proc = self.proc.take().expect("present during request");
                    let status = proc.reap(EXIT_GRACE);
                    let _ = proc.stderr_done.recv_timeout(STDERR_SETTLE);
                    let tail = proc.stderr_tail(6);
                    drop(proc);
                    self.loaded = None;
                    let oom = tail.to_lowercase().contains("memory");
                    let msg = format!(
                        "exit {:?}{}",
                        status.and_then(|s| s.code()),
                        if tail.is_empty() {
                            String::new()
                        } else {
                            format!(": {tail}")
                        }
                    );
                    return Err(if oom {
                        InferenceError::OutOfMemory(msg)
                    } else {
                        InferenceError::WorkerCrashed(msg)
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(deadline) = cancel_deadline {
                if Instant::now() > deadline {
                    self.kill();
                    return Err(InferenceError::Cancelled);
                }
            } else if cancelled() {
                let cancel = json!({"v": PROTOCOL_VERSION, "id": 0, "op": "cancel", "target": id});
                let proc = self.proc.as_mut().expect("present");
                let _ = proc.stdin.send(format!("{cancel}\n").into_bytes());
                cancel_deadline = Some(Instant::now() + self.cfg.cancel_grace);
            }
            if started.elapsed() > timeout {
                self.kill();
                return Err(InferenceError::Timeout(timeout.as_millis() as u64));
            }
        }
    }

    fn ensure_running(&mut self) -> Result<(), InferenceError> {
        if let Some(p) = self.proc.as_ref() {
            if !p.exited() {
                return Ok(());
            }
            self.kill();
            self.restarts += 1;
        }
        self.spawn()
    }

    fn memory_from(&self, v: &Value) -> BackendMemory {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let proc_mem = self.pid().and_then(hardware::process_memory);
        BackendMemory {
            active_bytes: n("active_bytes"),
            peak_bytes: n("peak_bytes"),
            cache_bytes: n("cache_bytes"),
            process_footprint_bytes: proc_mem.map(|m| m.phys_footprint_bytes),
            process_peak_footprint_bytes: proc_mem.map(|m| m.lifetime_max_phys_footprint_bytes),
        }
    }
}

impl InferenceBackend for MlxWorkerBackend {
    fn info(&self) -> BackendInfo {
        let h = self.hello.clone().unwrap_or_default();
        BackendInfo {
            backend: "mlx".into(),
            runtime_version: format!("mlx {} / mlx-vlm {}", h.mlx, h.mlx_vlm),
            accelerator: if h.metal { "metal" } else { "unknown" }.into(),
        }
    }

    fn ensure_loaded(
        &mut self,
        model: &VerifiedModel,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<LoadReport>, InferenceError> {
        self.ensure_running()?;
        if self.loaded.as_ref() == Some(model) {
            return Ok(None);
        }
        // Every (re)load re-hashes the store copy right before the worker
        // reads it: a worker respawned after a crash or an idle unload never
        // loads bytes changed since `model` was verified (about 1 s for a
        // 2 GB model, once per load, never per step).
        let fresh = ModelStore::new(self.cfg.models_root.clone())
            .verify(&model.spec)
            .map_err(|e| InferenceError::LoadFailed(e.to_string()))?;
        if fresh.dir != model.dir {
            return Err(InferenceError::LoadFailed(
                "the model is not in the model store".into(),
            ));
        }
        let dir = model
            .dir
            .to_str()
            .ok_or_else(|| InferenceError::BadRequest("model path is not UTF-8".into()))?;
        let mut load = json!({"op": "load", "model_dir": dir});
        if let Some(limit) = self.cfg.cache_limit_bytes {
            load["cache_limit_bytes"] = json!(limit);
        }
        let reply = self.request(load, self.cfg.load_timeout, cancelled);
        let reply = match reply {
            Ok(r) => r,
            Err(InferenceError::Protocol(m)) | Err(InferenceError::BadRequest(m)) => {
                return Err(InferenceError::LoadFailed(m))
            }
            Err(e) => return Err(e),
        };
        self.loaded = Some(model.clone());
        Ok(Some(LoadReport {
            model_id: model.spec.id.clone(),
            load_ms: reply.get("load_ms").and_then(Value::as_f64).unwrap_or(0.0),
            verify_ms: fresh.verify_ms,
            memory: self.memory_from(&reply),
        }))
    }

    fn generate(
        &mut self,
        req: &GenerateRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GenerateResponse, InferenceError> {
        if self.loaded.is_none() {
            return Err(InferenceError::BadRequest("no model is loaded".into()));
        }
        self.ensure_running()?;
        if self.loaded.is_none() {
            return Err(InferenceError::WorkerCrashed(
                "worker restarted; model must be reloaded".into(),
            ));
        }
        let engine = base64::engine::general_purpose::STANDARD;
        let images: Vec<Value> = req
            .images
            .iter()
            .map(|i| Value::String(engine.encode(i.png.as_slice())))
            .collect();
        let prep: Vec<Value> = req
            .images
            .iter()
            .map(|i| {
                let mut p = json!({});
                if let Some(c) = i.crop {
                    p["crop"] = json!(c);
                }
                if let Some((w, h)) = i.resize {
                    p["resize"] = json!([w, h]);
                }
                p
            })
            .collect();
        let started = Instant::now();
        let reply = self.request(
            json!({
                "op": "generate",
                "messages": wire_messages(&req.messages),
                "images": images,
                "image_prep": prep,
                "max_tokens": req.max_tokens,
                "temperature": req.temperature,
            }),
            req.timeout,
            cancelled,
        )?;
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let finish = reply
            .get("finish")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(20)
            .collect::<String>();
        if finish == "cancelled" {
            return Err(InferenceError::Cancelled);
        }
        let f = |p: &str| reply.pointer(p).and_then(Value::as_f64).unwrap_or(0.0);
        let u = |k: &str| reply.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        let image_sizes = reply
            .get("image_sizes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| Some((s.get(0)?.as_u64()? as u32, s.get(1)?.as_u64()? as u32)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(GenerateResponse {
            text: reply
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            finish,
            prompt_tokens: u("prompt_tokens"),
            generation_tokens: u("generation_tokens"),
            prompt_tps: f("/prompt_tps"),
            generation_tps: f("/generation_tps"),
            image_sizes,
            timings: Timings {
                image_ms: f("/timings_ms/image"),
                first_token_ms: f("/timings_ms/first_token"),
                generate_ms: f("/timings_ms/generate"),
                total_ms: f("/timings_ms/total"),
                wall_ms,
            },
            memory: self.memory_from(&reply),
        })
    }

    fn loaded_model_id(&self) -> Option<&str> {
        self.loaded_model()
    }

    fn process_id(&self) -> Option<i32> {
        self.pid()
    }

    fn memory(&mut self) -> Result<BackendMemory, InferenceError> {
        if self.proc.is_none() {
            return Ok(BackendMemory::default());
        }
        let reply = self.request(json!({"op": "stats"}), Duration::from_secs(10), &|| false)?;
        Ok(self.memory_from(&reply))
    }

    fn unload(&mut self) -> Result<(), InferenceError> {
        if self.proc.is_some() {
            self.request(json!({"op": "unload"}), Duration::from_secs(30), &|| false)?;
        }
        self.loaded = None;
        Ok(())
    }

    fn shutdown(&mut self) {
        if self.proc.is_some() {
            let _ = self.request(json!({"op": "shutdown"}), Duration::from_secs(3), &|| false);
            if let Some(mut p) = self.proc.take() {
                p.reap(Duration::from_secs(2));
            }
        }
        self.loaded = None;
    }
}

impl Drop for MlxWorkerBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{MAX_REPLY_BYTES, MAX_STDERR_LINES, MAX_STDERR_LINE_CHARS};
    use std::io::{BufRead, BufReader};

    /// A stand-in "worker" (shell script) exercises the supervisor:
    /// crashes, garbage, oversized replies and hangs are contained.
    fn fake_worker(tmp: &Path, body: &str) -> MlxWorkerConfig {
        let script = tmp.join("worker.sh");
        std::fs::write(&script, body).unwrap();
        let mut cfg = MlxWorkerConfig::new(PathBuf::from("/bin/sh"), script, tmp.to_path_buf());
        cfg.sandbox = false;
        cfg.hello_timeout = Duration::from_secs(5);
        cfg.cancel_grace = Duration::from_millis(300);
        cfg
    }

    // `/bin/sh -I` is not valid; the fake uses a wrapper that ignores it.
    fn sh_cfg(tmp: &Path, body: &str) -> MlxWorkerConfig {
        let wrapper = tmp.join("python");
        std::fs::write(&wrapper, "#!/bin/sh\nshift\nexec /bin/sh \"$@\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = fake_worker(tmp, body);
        cfg.python = wrapper;
        cfg
    }

    const HELLO: &str = r#"read line; echo '{"v":1,"id":1,"ok":true,"protocol":1,"worker_version":"t","mlx":"x","mlx_vlm":"y","metal":true}'"#;

    #[test]
    #[cfg(unix)]
    fn handshake_then_crash_is_reported_and_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(tmp.path(), &format!("{HELLO}\necho boom >&2\nexit 9\n"));
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        assert!(w.hello_info().unwrap().metal);
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(
            matches!(err, InferenceError::WorkerCrashed(ref m) if m.contains("boom")),
            "{err:?}"
        );
        assert!(w.pid().is_none());
        // Next use respawns.
        w.ensure_running().unwrap();
        assert!(w.pid().is_some());
    }

    #[test]
    #[cfg(unix)]
    fn garbage_and_oversized_replies_kill_the_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nread line\necho 'not json'\nsleep 5\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(matches!(err, InferenceError::Protocol(_)));
        assert!(w.pid().is_none());

        let big = "x".repeat(MAX_REPLY_BYTES + 10);
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nread line\nprintf '{{\"v\":1,\"id\":2,\"ok\":true,\"pad\":\"{big}\"}}\\n'\nsleep 5\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert!(matches!(err, InferenceError::Protocol(_)), "{err:?}");
    }

    #[test]
    #[cfg(unix)]
    fn hang_times_out_and_unhonored_cancel_kills() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nwhile read line; do :; done\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_millis(300), &|| {
                false
            })
            .unwrap_err();
        assert_eq!(err, InferenceError::Timeout(300));
        assert!(t.elapsed() < Duration::from_secs(3));
        assert!(w.pid().is_none());

        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(30), &|| true)
            .unwrap_err();
        assert_eq!(err, InferenceError::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[cfg(unix)]
    fn worker_errors_map_to_typed_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!(
                "{HELLO}\nread line\necho '{{\"v\":1,\"id\":2,\"ok\":false,\"error\":{{\"kind\":\"out_of_memory\",\"message\":\"metal oom\"}}}}'\nsleep 5\n"
            ),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "generate"}), Duration::from_secs(5), &|| false)
            .unwrap_err();
        assert_eq!(err, InferenceError::OutOfMemory("metal oom".into()));
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks whether the pid exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A worker that closes stdout but stays alive is killed and reaped
    /// within a bounded time, never waited on forever.
    #[test]
    #[cfg(unix)]
    fn closed_stdout_with_a_live_worker_is_killed_promptly() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!("{HELLO}\nread line\nexec 1>&-\nexec sleep 10\n"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let pid = w.pid().unwrap();
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(30), &|| false)
            .unwrap_err();
        assert!(matches!(err, InferenceError::WorkerCrashed(_)), "{err:?}");
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(w.pid().is_none());
        assert!(!alive(pid));
    }

    /// A worker that stops reading stdin cannot hold a request past its
    /// timeout or a cancel (generate lines carry megabytes of images, far
    /// more than a pipe buffer).
    #[test]
    #[cfg(unix)]
    fn worker_that_stops_reading_cannot_block_a_request() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(tmp.path(), &format!("{HELLO}\nexec sleep 10\n"));
        let big = json!({"op": "stats", "pad": "x".repeat(4 << 20)});
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(big.clone(), Duration::from_millis(300), &|| false)
            .unwrap_err();
        assert_eq!(err, InferenceError::Timeout(300));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(w.pid().is_none());

        w.spawn().unwrap();
        let t = Instant::now();
        let err = w
            .request(big, Duration::from_secs(30), &|| true)
            .unwrap_err();
        assert_eq!(err, InferenceError::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }

    /// Invalid UTF-8 and an over-long line on stderr neither stop the
    /// drain nor reach the error message unbounded.
    #[test]
    #[cfg(unix)]
    fn hostile_stderr_keeps_draining_and_stays_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = sh_cfg(
            tmp.path(),
            &format!(
                "{HELLO}\nread line\nprintf 'bad \\377\\376 bytes\\n' >&2\n\
                 head -c 3000000 /dev/zero | tr '\\0' x >&2\necho >&2\necho boom >&2\nexit 9\n"
            ),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_secs(10), &|| false)
            .unwrap_err();
        let InferenceError::WorkerCrashed(m) = err else {
            panic!("{err:?}")
        };
        assert!(m.contains("boom") && m.contains("bad \u{FFFD}"), "{m}");
        assert!(
            m.chars().count() < 6 * (MAX_STDERR_LINE_CHARS + 3) + 40,
            "{m}"
        );
    }

    #[test]
    fn stderr_drain_bounds_lines_and_survives_any_bytes() {
        let mut input = vec![b'x'; 5 << 20];
        input.extend_from_slice(b"\nbad \xff\xfe bytes\nafter\npartial at eof");
        let sink = Mutex::new(VecDeque::new());
        drain_stderr(input.as_slice(), &sink);
        let q = sink.into_inner().unwrap();
        assert_eq!(q.len(), 4);
        assert_eq!(q[0], "x".repeat(MAX_STDERR_LINE_CHARS));
        assert_eq!(q[1], "bad \u{FFFD}\u{FFFD} bytes");
        assert_eq!(q[2], "after");
        assert_eq!(q[3], "partial at eof");

        let many: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let sink = Mutex::new(VecDeque::new());
        drain_stderr(many.as_bytes(), &sink);
        let q = sink.into_inner().unwrap();
        assert_eq!(q.len(), MAX_STDERR_LINES);
        assert_eq!(q.back().unwrap(), "line 99");
    }

    /// A temp dir tree next to an `outside` directory that must survive
    /// any cleanup: files, nested dirs and symlinks to `outside` inside.
    fn temp_tree(root: &Path) -> (PathBuf, PathBuf) {
        let dir = root.join("pegoles-mlx");
        let outside = root.join("outside");
        std::fs::create_dir_all(dir.join("nested/deeper")).unwrap();
        std::fs::create_dir_all(outside.join("sub")).unwrap();
        std::fs::write(dir.join("a"), "x").unwrap();
        std::fs::write(dir.join("nested/deeper/b"), "x").unwrap();
        std::fs::write(outside.join("keep"), "x").unwrap();
        std::fs::write(outside.join("sub/keep"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("nested/link")).unwrap();
        (dir, outside)
    }

    fn outside_intact(outside: &Path) -> bool {
        outside.join("keep").is_file() && outside.join("sub/keep").is_file()
    }

    #[test]
    fn clearing_the_temp_dir_removes_everything_and_follows_no_link() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, outside) = temp_tree(tmp.path());
        clear_dir(open_private_dir(&dir).unwrap(), 0, &mut ClearBudget::new());
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert!(outside_intact(&outside));
    }

    /// The temp dir is swapped for a symlink to an outside directory after
    /// it was checked (by the worker, which may write to the directory
    /// itself, or by something it forked that outlived it): cleanup goes
    /// through the descriptor, empties the directory that was checked and
    /// leaves the link's target alone; the next spawn refuses the link.
    #[test]
    fn a_temp_dir_swapped_for_a_symlink_is_never_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, outside) = temp_tree(tmp.path());
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fd = open_private_dir(&dir).unwrap();
        let moved = tmp.path().join("moved");
        std::fs::rename(&dir, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        clear_dir(fd, 0, &mut ClearBudget::new());
        assert_eq!(std::fs::read_dir(&moved).unwrap().count(), 0);
        assert!(outside_intact(&outside));
        assert!(open_private_dir(&dir).is_err());
        assert!(outside_intact(&outside));
        let mode = std::fs::metadata(&outside).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the link target was re-permissioned");
    }

    /// Entries swapped between a directory and a symlink to an outside
    /// directory while cleanup runs never lead cleanup outside.
    #[test]
    fn entries_swapped_during_cleanup_are_never_followed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let tmp = tempfile::tempdir().unwrap();
        for _ in 0..20 {
            let (dir, outside) = temp_tree(tmp.path());
            for i in 0..200 {
                std::fs::write(dir.join(format!("nested/f{i}")), "x").unwrap();
            }
            let stop = Arc::new(AtomicBool::new(false));
            let swapper = {
                let (dir, outside, stop) = (dir.clone(), outside.clone(), stop.clone());
                std::thread::spawn(move || {
                    let (real, parked) = (dir.join("nested"), dir.join("parked"));
                    while !stop.load(Ordering::Relaxed) {
                        if std::fs::rename(&real, &parked).is_ok() {
                            let _ = std::os::unix::fs::symlink(&outside, &real);
                            let _ = std::fs::remove_file(&real);
                            let _ = std::fs::rename(&parked, &real);
                        }
                    }
                })
            };
            clear_dir(open_private_dir(&dir).unwrap(), 0, &mut ClearBudget::new());
            stop.store(true, Ordering::Relaxed);
            swapper.join().unwrap();
            assert!(outside_intact(&outside));
            std::fs::remove_dir_all(&dir).unwrap();
            std::fs::remove_dir_all(&outside).unwrap();
        }
    }

    /// Directories made unreadable are still removed; nesting past
    /// `MAX_CLEAR_DEPTH` is left alone without stopping the rest.
    #[test]
    fn clearing_handles_locked_and_too_deep_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pegoles-mlx");
        std::fs::create_dir_all(dir.join("locked/inner")).unwrap();
        std::fs::write(dir.join("locked/inner/f"), "x").unwrap();
        for p in ["locked/inner", "locked"] {
            std::fs::set_permissions(dir.join(p), std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let mut deep = dir.join("deep");
        for _ in 0..MAX_CLEAR_DEPTH + 4 {
            deep.push("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.join("z"), "x").unwrap();
        clear_dir(open_private_dir(&dir).unwrap(), 0, &mut ClearBudget::new());
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["deep"]);
    }

    /// The sandbox's TMP parameter resolves the temp dir's parent only: a
    /// temp dir swapped for a symlink never grants writes to its target.
    #[test]
    fn sandbox_tmp_param_never_resolves_the_temp_dir_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let dir = tmp.path().join("pegoles-mlx");
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        let cfg = MlxWorkerConfig::new(
            PathBuf::from("/bin/sh"),
            tmp.path().join("worker.py"),
            tmp.path().to_path_buf(),
        );
        let args = cfg.sandbox_args(&dir).unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let want = format!("TMP={}", root.join("pegoles-mlx").display());
        assert!(args.contains(&want), "{args:?}");
    }

    /// The guard behind a failed spawn (e.g. a reader thread that cannot
    /// start): dropping a `Proc` kills its process group and reaps it.
    #[test]
    #[cfg(unix)]
    fn dropping_a_proc_kills_and_reaps_the_worker() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 30 & echo $!; exec sleep 30"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let forked: i32 = line.trim().parse().unwrap();
        let (stdin, _) = mpsc::channel();
        let (_, rx) = mpsc::channel();
        let (_, stderr_done) = mpsc::channel();
        let proc = Proc {
            child,
            stdin,
            rx,
            stderr: Arc::default(),
            stderr_done,
            pid,
            reaped: false,
        };
        assert!(alive(pid) && alive(forked));
        assert!(!proc.exited());
        drop(proc);
        assert!(!alive(pid));
        assert!(gone_soon(forked), "a forked process outlived the worker");
    }

    /// Whether `pid` disappears within a few seconds (a killed orphan is
    /// reaped by launchd/init, not by us).
    fn gone_soon(pid: i32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
        true
    }

    /// A process the worker forked keeps the worker's stdout open, so the
    /// supervisor sees no EOF: the request still ends at its timeout, and
    /// killing the worker's group ends the forked process too.
    #[test]
    #[cfg(unix)]
    fn a_forked_process_does_not_outlive_the_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("forked.pid");
        let cfg = sh_cfg(
            tmp.path(),
            &format!(
                "{HELLO}\nsleep 30 &\necho $! > '{}'\nexec 1>&-\nexec sleep 30\n",
                pidfile.display()
            ),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        w.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let forked = loop {
            let text = std::fs::read_to_string(&pidfile).unwrap_or_default();
            if let Ok(pid) = text.trim().parse::<i32>() {
                break pid;
            }
            assert!(Instant::now() < deadline, "no pid file");
            std::thread::sleep(POLL);
        };
        assert!(alive(forked));
        let t = Instant::now();
        let err = w
            .request(json!({"op": "stats"}), Duration::from_millis(300), &|| {
                false
            })
            .unwrap_err();
        assert_eq!(err, InferenceError::Timeout(300));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(gone_soon(forked), "a forked process outlived the worker");
    }

    const ECHO_OK: &str =
        r#"n=2; while read line; do echo "{\"v\":1,\"id\":$n,\"ok\":true}"; n=$((n+1)); done"#;

    /// Every (re)load re-hashes the model first: bytes changed since the
    /// caller verified them are never loaded by a respawned worker.
    #[test]
    #[cfg(unix)]
    fn reload_after_a_restart_reverifies_the_model_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let config = br#"{"model_type":"qwen3_vl"}"#;
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("config.json"), config).unwrap();
        std::fs::write(src.join("model.safetensors"), b"weights!").unwrap();
        let spec = crate::store::tests::spec(&[
            ("config.json", config),
            ("model.safetensors", b"weights!"),
        ]);
        let models = tmp.path().join("models");
        let model = crate::store::ModelStore::new(models.clone())
            .import_local(&spec, &src)
            .unwrap();
        let mut cfg = sh_cfg(tmp.path(), &format!("{HELLO}\n{ECHO_OK}\n"));
        cfg.models_root = models;
        let mut w = MlxWorkerBackend::new(cfg);
        assert!(w.ensure_loaded(&model, &|| false).unwrap().is_some());
        assert!(w.ensure_loaded(&model, &|| false).unwrap().is_none());
        // Same size, other bytes; then the worker goes away (a crash or
        // an idle unload) and the model must be loaded again.
        std::fs::write(model.dir.join("model.safetensors"), b"WEIGHTS!").unwrap();
        w.kill();
        let err = w.ensure_loaded(&model, &|| false).unwrap_err();
        assert!(
            matches!(err, InferenceError::LoadFailed(ref m) if m.contains("checksum")),
            "{err:?}"
        );
        assert!(w.loaded_model().is_none());
    }

    /// Replies go to a private copy of fd 1: stray output from libraries
    /// (a Python print, a native write to fd 1) lands on stderr and never
    /// corrupts the protocol stream. Needs only the runtime's Python.
    const STRAY_OUTPUT: &str = r#"
import importlib.util, os, sys
spec = importlib.util.spec_from_file_location("pegoles_mlx_worker", sys.argv[1])
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
print("stray print", flush=True)
os.write(1, b"stray native write\n")
worker.reply(7, x=1)
"#;

    #[test]
    fn stray_stdout_output_cannot_corrupt_the_protocol() {
        let python = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/pegoles-runtime/python/bin/python3.12");
        // The runtime is a macOS (Apple silicon) build.
        if !cfg!(target_os = "macos") || !python.is_file() {
            eprintln!("skipped: build the runtime with scripts/local-model/build-runtime.sh");
            return;
        }
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workers/mlx/pegoles_mlx_worker.py");
        // -B: importing the worker must not leave bytecode in the repo.
        let out = Command::new(&python)
            .args(["-I", "-B", "-c", STRAY_OUTPUT])
            .arg(&script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "{\"v\":1,\"id\":7,\"ok\":true,\"x\":1}\n"
        );
        assert!(
            stderr.contains("stray print") && stderr.contains("stray native write"),
            "{stderr}"
        );
    }

    /// Escape attempts from inside the real sandbox profile, run with the
    /// real runtime built by `scripts/local-model/build-runtime.sh` (skipped
    /// when it has not been built, e.g. in CI). Every probe is first run
    /// unsandboxed as a control (it must work there, so "blocked" really
    /// means the sandbox), then under the worker profile, where it must
    /// fail; Metal and the private temp dir must keep working.
    const ESCAPE_PROBES: &str = r#"
import ctypes, json, os, socket, subprocess, sys
parent, port, home_dir, outside, private = sys.argv[1:6]
parent, port = int(parent), int(port)
res = {}
def probe(name, fn):
    try:
        fn()
        res[name] = "allowed"
    except Exception:
        res[name] = "blocked"
libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
def lookup(service):
    port = ctypes.c_uint32(0)
    bp = ctypes.c_uint32.in_dll(libc, "bootstrap_port")
    if libc.bootstrap_look_up(bp, service.encode(), ctypes.byref(port)) != 0:
        raise OSError("lookup failed")
def pidpath(pid):
    buf = ctypes.create_string_buffer(4096)
    if libc.proc_pidpath(pid, buf, 4096) <= 0:
        raise OSError("proc_pidpath failed")
def write(path):
    with open(path, "w") as f:
        f.write("x")
    os.unlink(path)
def metal():
    import mlx.core as mx
    x = (mx.arange(4096, dtype=mx.float32) * 3 + 1).sum()
    mx.eval(x)
    assert mx.metal.is_available() and x.item() > 0
def fork():
    pid = os.fork()
    if pid == 0:
        os._exit(0)
    os.waitpid(pid, 0)
probe("fork", fork)
probe("exec", lambda: subprocess.run(["/usr/bin/true"], check=True))
probe("tcp_loopback", lambda: socket.create_connection(("127.0.0.1", port), timeout=3).close())
probe("dns_socket", lambda: socket.socket(socket.AF_UNIX).connect("/var/run/mDNSResponder"))
probe("read_home", lambda: open(os.path.join(home_dir, "canary"), "rb").read())
probe("write_home", lambda: write(os.path.join(home_dir, "written")))
probe("write_shared_tmp", lambda: write(os.path.join(outside, "pegoles-sbx-probe-%d" % os.getpid())))
probe("launchservices", lambda: lookup("com.apple.coreservices.launchservicesd"))
probe("pasteboard", lambda: lookup("com.apple.pasteboard.1"))
probe("securityd", lambda: lookup("com.apple.SecurityServer"))
probe("securityd_xpc", lambda: lookup("com.apple.securityd.xpc"))
probe("lsd_open", lambda: lookup("com.apple.lsd.open"))
probe("appleevents", lambda: lookup("com.apple.coreservices.appleevents"))
probe("windowserver", lambda: lookup("com.apple.windowserver.active"))
probe("cfprefsd", lambda: lookup("com.apple.cfprefsd.daemon"))
probe("signal_parent", lambda: os.kill(parent, 0))
probe("inspect_parent", lambda: pidpath(parent))
probe("metal", metal)
probe("write_private_tmp", lambda: write(os.path.join(private, "ok-%d" % os.getpid())))
print(json.dumps(res))
"#;

    const ESCAPES: [&str; 17] = [
        "fork",
        "exec",
        "tcp_loopback",
        "dns_socket",
        "read_home",
        "write_home",
        "write_shared_tmp",
        "launchservices",
        "pasteboard",
        "securityd",
        "securityd_xpc",
        "lsd_open",
        "appleevents",
        "windowserver",
        "cfprefsd",
        "signal_parent",
        "inspect_parent",
    ];

    #[test]
    fn sandbox_blocks_escapes() {
        let python = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/pegoles-runtime/python/bin/python3.12");
        if !python.is_file() || !Path::new(SANDBOX_EXEC).exists() {
            eprintln!("skipped: build the runtime with scripts/local-model/build-runtime.sh");
            return;
        }
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        // A canary directory under HOME (outside every allowed subpath).
        let home_dir = tempfile::tempdir_in(&home).unwrap();
        std::fs::write(home_dir.path().join("canary"), "secret").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let models = tempfile::tempdir().unwrap();
        let script = models.path().join("worker.py");
        std::fs::write(&script, "").unwrap();
        let cfg = MlxWorkerConfig::new(python.clone(), script, models.path().to_path_buf());
        let (tmp, _) = worker_tmp_dir().unwrap();
        let outside = tmp.parent().unwrap().to_path_buf();
        let run = |sandboxed: bool| -> Value {
            let mut cmd = if sandboxed {
                let mut c = Command::new(SANDBOX_EXEC);
                c.args(cfg.sandbox_args(&tmp).unwrap()).arg(&python);
                c
            } else {
                Command::new(&python)
            };
            let out = cmd
                .args(["-I", "-c", ESCAPE_PROBES])
                .arg(std::process::id().to_string())
                .arg(&port)
                .arg(home_dir.path())
                .arg(&outside)
                .arg(&tmp)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TMPDIR", &tmp)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "probe script failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            serde_json::from_slice(&out.stdout).unwrap()
        };
        let control = run(false);
        let boxed = run(true);
        for probe in ESCAPES {
            assert_eq!(control[probe], "allowed", "control {probe}: {control}");
            assert_eq!(
                boxed[probe], "blocked",
                "{probe} escaped the sandbox: {boxed}"
            );
        }
        assert_eq!(boxed["metal"], "allowed", "{boxed}");
        assert_eq!(boxed["write_private_tmp"], "allowed", "{boxed}");
        assert!(!home_dir.path().join("written").exists());
    }

    #[test]
    fn missing_runtime_is_a_clear_error() {
        let cfg = MlxWorkerConfig::new(
            PathBuf::from("/nonexistent/python"),
            PathBuf::from("x"),
            PathBuf::from("/nonexistent"),
        );
        let mut w = MlxWorkerBackend::new(cfg);
        assert!(matches!(w.spawn(), Err(InferenceError::RuntimeMissing(_))));
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("worker.py");
        std::fs::write(&script, "").unwrap();
        // No interpreter at any candidate: a clear error, never a fallback.
        assert!(matches!(
            MlxWorkerConfig::discover_from(
                &[tmp.path().join("runtime/python/bin/python3.12")],
                std::slice::from_ref(&script),
                tmp.path(),
            ),
            Err(InferenceError::RuntimeMissing(_))
        ));
        // An interpreter but no worker script: also refused.
        let python = tmp.path().join("python3.12");
        std::fs::write(&python, "").unwrap();
        assert!(matches!(
            MlxWorkerConfig::discover_from(
                std::slice::from_ref(&python),
                &[tmp.path().join("missing.py")],
                tmp.path(),
            ),
            Err(InferenceError::RuntimeMissing(_))
        ));
    }
}

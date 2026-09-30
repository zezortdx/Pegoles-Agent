//! Host-side endpoint of the egress stream (see `docs/EGRESS.md`).
//!
//! The app creates a private local endpoint FIRST, then asks the VM host
//! helper (`egress_open`) to connect to it; the helper bridges the raw
//! bytes to the guest's forwarder over vsock/HvSocket. The app accepts
//! exactly one connection (10 s), then the endpoint disappears, so nothing
//! else can ever attach to it.
//!
//! - unix: a Unix socket `<data root>/egress/<id>/s` in a fresh 0700
//!   directory. The socket is NOT placed in the computer folder: macOS
//!   limits `sun_path` to 104 bytes and `<data>/computers/<uuid>/…` alone
//!   exceeds it for ordinary user names.
//! - windows: a named pipe `\\.\pipe\pegoles-egress-<uuid>` with a
//!   current-user-only DACL, `PIPE_REJECT_REMOTE_CLIENTS`, first instance.
//!
//! SECURITY: transport only. The stream's content is untrusted guest
//! bytes; the mux/policy lives in `pegoles-egress`.

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(unix)]
use crate::error::{ComputerError, Result};

#[cfg(windows)]
pub mod win;

/// Canonical port lives in Rust (`pegoles-egress-proto::EGRESS_VSOCK_PORT`);
/// duplicated here (and in Swift `EgressBridge.swift`) because this crate
/// does not depend on it. Keep in sync.
pub const EGRESS_VSOCK_PORT: u32 = 4051;

/// How long the app waits for the helper's connection to its endpoint.
pub const EGRESS_ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the VM host helper may take to answer an egress open.
pub const EGRESS_HELPER_TIMEOUT: Duration = Duration::from_secs(12);
/// How long closing the egress stream waits for the helper.
pub const EGRESS_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// `sockaddr_un.sun_path` holds 104 bytes on macOS (incl. NUL).
pub const UNIX_SOCKET_PATH_MAX: usize = 103;
/// Directory under the data root that holds per-session endpoint dirs.
pub const EGRESS_DIR_NAME: &str = "egress";
/// Socket file name inside the per-session directory.
pub const EGRESS_SOCKET_NAME: &str = "s";
/// Windows pipe name prefix (the suffix is a lowercase UUID).
pub const EGRESS_PIPE_PREFIX: &str = r"\\.\pipe\pegoles-egress-";

/// Whether `name` is exactly `\\.\pipe\pegoles-egress-<lowercase uuid>`.
pub fn is_egress_pipe_name(name: &str) -> bool {
    name.strip_prefix(EGRESS_PIPE_PREFIX)
        .is_some_and(is_lower_uuid)
}

fn is_lower_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// Whether `path` is exactly `<data_root>/egress/<1..=40 of [0-9a-f-]>/s`,
/// absolute, without `..`, and short enough for `sun_path`. The Swift
/// helper applies the same rule (`validatedEgressEndpoint`); keep in sync.
pub fn is_egress_socket_path(data_root: &Path, path: &Path) -> bool {
    use std::path::Component;
    let Some(text) = path.to_str() else {
        return false;
    };
    if !path.is_absolute() || text.len() > UNIX_SOCKET_PATH_MAX || text.contains('\0') {
        return false;
    }
    let Ok(rest) = path.strip_prefix(data_root.join(EGRESS_DIR_NAME)) else {
        return false;
    };
    let mut parts = rest.components();
    let (Some(Component::Normal(dir)), Some(Component::Normal(file)), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let dir = dir.to_str().unwrap_or("");
    file == EGRESS_SOCKET_NAME
        && (1..=40).contains(&dir.len())
        && dir
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// The waiting half of an egress open (see `ComputerBackend::begin_open_egress`).
/// It owns everything it needs, so it runs without the caller's lock.
pub trait PendingEgressOpen: Send {
    /// Wait for the helper's answer and the guest's connection (bounded by
    /// the egress timeouts), undoing the open on failure.
    fn wait(self: Box<Self>) -> crate::error::Result<EgressEndpoint>;
}

/// An open that already finished (backends with nothing to wait for).
pub struct ReadyEgress(pub EgressEndpoint);

impl PendingEgressOpen for ReadyEgress {
    fn wait(self: Box<Self>) -> crate::error::Result<EgressEndpoint> {
        Ok(self.0)
    }
}

/// The accepted egress connection. Plain `Read + Write`; the egress crate
/// adapts it to tokio (`UnixStream::from_std` on unix;
/// `NamedPipeServer::from_raw_handle` on Windows, whose handle is opened
/// for overlapped I/O).
#[derive(Debug)]
pub enum EgressEndpoint {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Pipe(win::PipeHandle),
}

impl EgressEndpoint {
    /// A second handle to the same stream (one per direction).
    pub fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.try_clone().map(Self::Unix),
            #[cfg(windows)]
            Self::Pipe(p) => p.try_clone().map(Self::Pipe),
        }
    }

    /// Close both directions, waking blocked readers/writers.
    pub fn shutdown(&self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.shutdown(std::net::Shutdown::Both),
            #[cfg(windows)]
            Self::Pipe(p) => {
                p.cancel();
                Ok(())
            }
        }
    }

    /// The underlying Unix stream (for `tokio::net::UnixStream::from_std`).
    #[cfg(unix)]
    pub fn into_unix(self) -> UnixStream {
        let Self::Unix(s) = self;
        s
    }

    /// The underlying overlapped pipe handle (for tokio's named pipes).
    #[cfg(windows)]
    pub fn into_pipe(self) -> win::PipeHandle {
        let Self::Pipe(p) = self;
        p
    }
}

impl Read for EgressEndpoint {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
            #[cfg(windows)]
            Self::Pipe(p) => p.read(buf),
        }
    }
}

impl Write for EgressEndpoint {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
            #[cfg(windows)]
            Self::Pipe(p) => p.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) use unix_endpoint::PendingEndpoint;
#[cfg(windows)]
pub(crate) use win::PendingEndpoint;

#[cfg(unix)]
mod unix_endpoint {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime};

    /// Leftovers of a crashed session older than this are swept.
    const STALE_AFTER: Duration = Duration::from_secs(120);

    /// A listening endpoint nobody has connected to yet. Dropping it (or
    /// accepting) removes the socket and its directory.
    pub(crate) struct PendingEndpoint {
        listener: UnixListener,
        dir: PathBuf,
        sock: PathBuf,
    }

    fn backend(what: &str, e: impl std::fmt::Display) -> ComputerError {
        ComputerError::Backend(format!("egress endpoint: {what}: {e}"))
    }

    fn private_dir(path: &Path, recursive: bool) -> Result<()> {
        std::fs::DirBuilder::new()
            .recursive(recursive)
            .mode(0o700)
            .create(path)
            .map_err(|e| backend("cannot create a private folder", e))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| backend("cannot restrict the folder", e))
    }

    fn sweep_stale(base: &Path) {
        let Ok(entries) = std::fs::read_dir(base) else {
            return;
        };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > STALE_AFTER);
            if old && entry.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    impl PendingEndpoint {
        pub(crate) fn create(data_root: &Path) -> Result<Self> {
            let base = data_root.join(EGRESS_DIR_NAME);
            if !base.is_dir() {
                private_dir(&base, true)?;
            }
            sweep_stale(&base);
            let name = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
            let dir = base.join(name);
            private_dir(&dir, false)?;
            let sock = dir.join(EGRESS_SOCKET_NAME);
            let mut pending = None;
            if is_egress_socket_path(data_root, &sock) {
                match UnixListener::bind(&sock) {
                    Ok(listener) => {
                        pending = Some(Self {
                            listener,
                            dir: dir.clone(),
                            sock: sock.clone(),
                        });
                    }
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(&dir);
                        return Err(backend("cannot listen", e));
                    }
                }
            } else {
                let _ = std::fs::remove_dir_all(&dir);
            }
            let pending = pending.ok_or_else(|| {
                ComputerError::Backend(
                    "egress endpoint: the data folder path is too long for a local socket"
                        .to_string(),
                )
            })?;
            std::fs::set_permissions(&pending.sock, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| backend("cannot restrict the socket", e))?;
            Ok(pending)
        }

        /// What goes into `egress_open.endpoint`.
        pub(crate) fn wire_endpoint(&self) -> String {
            self.sock.to_string_lossy().into_owned()
        }

        /// Accept exactly one connection within `timeout`; the endpoint is
        /// removed either way.
        pub(crate) fn accept(self, timeout: Duration) -> Result<EgressEndpoint> {
            self.listener
                .set_nonblocking(true)
                .map_err(|e| backend("cannot poll", e))?;
            let deadline = Instant::now() + timeout;
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        // The accepted socket inherits O_NONBLOCK on BSD.
                        stream
                            .set_nonblocking(false)
                            .map_err(|e| backend("cannot configure the stream", e))?;
                        if !peer_is_this_user(&stream) {
                            return Err(ComputerError::Backend(
                                "egress endpoint: the connecting process is not this user"
                                    .to_string(),
                            ));
                        }
                        return Ok(EgressEndpoint::Unix(stream));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return Err(ComputerError::Timeout(
                                "the helper did not connect to the egress endpoint".to_string(),
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(backend("accept failed", e)),
                }
            }
        }
    }

    impl Drop for PendingEndpoint {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.sock);
            let _ = std::fs::remove_dir(&self.dir);
        }
    }

    #[cfg(target_os = "macos")]
    fn peer_is_this_user(stream: &UnixStream) -> bool {
        use std::os::fd::AsRawFd;
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: valid fd; out-params are ours.
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        // SAFETY: plain getter.
        rc == 0 && uid == unsafe { libc::geteuid() }
    }

    /// Other unix hosts rely on the 0700 directory alone.
    #[cfg(not(target_os = "macos"))]
    fn peer_is_this_user(_: &UnixStream) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const UUID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn pipe_names_are_exactly_the_egress_form() {
        assert!(is_egress_pipe_name(&format!("{EGRESS_PIPE_PREFIX}{UUID}")));
        for bad in [
            String::new(),
            format!(r"\\.\pipe\pegoles-serial-{UUID}"),
            format!(r"\\.\pipe\pegoles-egress-{}", UUID.to_uppercase()),
            format!(r"\\.\pipe\pegoles-egress-{UUID}\x"),
            format!(r"\\.\pipe\pegoles-egress-{UUID}0"),
            format!(r"\\.\pipe\pegoles-egress-{}", &UUID[1..]),
            format!(r"\\.\pipe\pegoles-egress-..\{UUID}"),
            format!(r"\\server\pipe\pegoles-egress-{UUID}"),
            format!(r"//./pipe/pegoles-egress-{UUID}"),
            format!(r"pegoles-egress-{UUID}"),
        ] {
            assert!(!is_egress_pipe_name(&bad), "{bad}");
        }
    }

    #[test]
    fn socket_paths_must_sit_in_the_egress_folder_of_the_data_root() {
        let root = Path::new("/data/Pegoles");
        let good = root.join("egress/0123456789abcdef/s");
        assert!(is_egress_socket_path(root, &good));
        for bad in [
            "/data/Pegoles/egress/0123456789abcdef/x",
            "/data/Pegoles/egress/0123456789abcdef/s/extra",
            "/data/Pegoles/egress/s",
            "/data/Pegoles/egress/../computers/0123/s",
            "/data/Pegoles/egress/0123456789abcdef/../s",
            "/data/Pegoles/egress/UPPER/s",
            "/data/Pegoles/egress//s",
            "/data/Pegoles/computers/0123456789abcdef/s",
            "/data/Pegoles/egress-x/0123456789abcdef/s",
            "/tmp/egress/0123456789abcdef/s",
            "egress/0123456789abcdef/s",
            "/data/Pegoles/egress/0123456789abcdef/s\0",
        ] {
            assert!(!is_egress_socket_path(root, Path::new(bad)), "{bad}");
        }
        let long = Path::new("/data/Pegoles").join(format!("egress/{}/s", "a".repeat(41)));
        assert!(!is_egress_socket_path(root, &long));
        let deep_root = PathBuf::from(format!("/{}", "r".repeat(95)));
        assert!(!is_egress_socket_path(
            &deep_root,
            &deep_root.join("egress/0123456789abcdef/s")
        ));
    }

    #[test]
    fn the_vm_never_gets_a_network_device() {
        // The egress stream is a virtio-socket stream, not a NIC. Source
        // guard for the macOS helper (its runtime check refuses any VM
        // configuration with `networkDevices`).
        for (name, src) in [
            (
                "VmManager.swift",
                include_str!("../../../native/macos/pegoles-vm-host/Sources/VmManager.swift"),
            ),
            (
                "EgressBridge.swift",
                include_str!("../../../native/macos/pegoles-vm-host/Sources/EgressBridge.swift"),
            ),
        ] {
            let code: String = src
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for forbidden in [
                "VZNetworkDeviceConfiguration",
                "VZVirtioNetworkDeviceConfiguration",
                "VZNATNetworkDeviceAttachment",
                "VZBridgedNetworkDeviceAttachment",
                "VZFileHandleNetworkDeviceAttachment",
                "networkDevices =",
                "networkDevices.append",
            ] {
                assert!(!code.contains(forbidden), "{name} mentions {forbidden}");
            }
        }
        let vm = include_str!("../../../native/macos/pegoles-vm-host/Sources/VmManager.swift");
        assert!(vm.contains("config.networkDevices.isEmpty"));
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixStream;

        fn root() -> tempfile::TempDir {
            // Short prefix: macOS sun_path is 104 bytes.
            tempfile::Builder::new().prefix("pg").tempdir().unwrap()
        }

        fn mode(path: &Path) -> u32 {
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn endpoint_is_private_and_matches_the_helper_rule() {
            let root = root();
            let pending = PendingEndpoint::create(root.path()).unwrap();
            let sock = PathBuf::from(pending.wire_endpoint());
            assert!(is_egress_socket_path(root.path(), &sock));
            assert_eq!(mode(&root.path().join("egress")), 0o700);
            assert_eq!(mode(sock.parent().unwrap()), 0o700);
            assert_eq!(mode(&sock) & 0o077, 0, "socket is owner-only");
            use std::os::unix::fs::FileTypeExt;
            assert!(std::fs::metadata(&sock).unwrap().file_type().is_socket());
            drop(pending);
            assert!(!sock.exists() && !sock.parent().unwrap().exists());
        }

        #[test]
        fn accepts_exactly_one_connection_then_the_endpoint_is_gone() {
            let root = root();
            let pending = PendingEndpoint::create(root.path()).unwrap();
            let sock = PathBuf::from(pending.wire_endpoint());
            let mut first = UnixStream::connect(&sock).unwrap();
            // A second client queued behind the first is never served.
            let mut second = UnixStream::connect(&sock).unwrap();
            let mut accepted = pending.accept(Duration::from_secs(2)).unwrap();
            assert!(!sock.exists(), "endpoint removed after accept");
            assert!(UnixStream::connect(&sock).is_err());
            first.write_all(b"ping").unwrap();
            let mut buf = [0u8; 4];
            accepted.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"ping");
            accepted.write_all(b"pong").unwrap();
            first.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"pong");
            // (may already be reset by the dropped listener)
            let _ = second.set_read_timeout(Some(Duration::from_secs(2)));
            let mut byte = [0u8; 1];
            assert!(
                !matches!(second.read(&mut byte), Ok(n) if n > 0),
                "the second client gets nothing"
            );
        }

        #[test]
        fn accept_times_out_and_cleans_up() {
            let root = root();
            let pending = PendingEndpoint::create(root.path()).unwrap();
            let sock = PathBuf::from(pending.wire_endpoint());
            let started = std::time::Instant::now();
            let err = pending.accept(Duration::from_millis(150)).unwrap_err();
            assert!(matches!(err, ComputerError::Timeout(_)), "{err:?}");
            assert!(started.elapsed() < Duration::from_secs(3));
            assert!(!sock.exists() && !sock.parent().unwrap().exists());
        }

        #[test]
        fn sessions_get_distinct_folders_and_stale_ones_are_swept() {
            let root = root();
            let a = PendingEndpoint::create(root.path()).unwrap();
            let b = PendingEndpoint::create(root.path()).unwrap();
            assert_ne!(a.wire_endpoint(), b.wire_endpoint());
            // A crashed session's leftover, made old.
            let stale = root.path().join("egress/deadbeefdeadbeef");
            std::fs::create_dir(&stale).unwrap();
            let old = std::time::SystemTime::now() - Duration::from_secs(3600);
            std::fs::File::open(&stale)
                .unwrap()
                .set_modified(old)
                .unwrap();
            let c = PendingEndpoint::create(root.path()).unwrap();
            assert!(!stale.exists());
            assert!(Path::new(&a.wire_endpoint()).exists(), "live ones stay");
            drop((a, b, c));
        }

        #[test]
        fn a_path_too_long_for_a_socket_fails_closed() {
            let root = tempfile::Builder::new()
                .prefix(&"x".repeat(90))
                .tempdir()
                .unwrap();
            let err = PendingEndpoint::create(root.path()).err().unwrap();
            assert!(err.to_string().contains("too long"), "{err}");
            let left = std::fs::read_dir(root.path().join("egress"))
                .map(|d| d.count())
                .unwrap_or(0);
            assert_eq!(left, 0, "no leftover folder");
        }

        #[test]
        fn endpoint_clone_and_shutdown() {
            let (a, b) = UnixStream::pair().unwrap();
            let ep = EgressEndpoint::Unix(a);
            let mut twin = ep.try_clone().unwrap();
            let mut peer = b;
            twin.write_all(b"x").unwrap();
            let mut byte = [0u8; 1];
            peer.read_exact(&mut byte).unwrap();
            ep.shutdown().unwrap();
            assert_eq!(peer.read(&mut byte).unwrap(), 0);
        }
    }
}

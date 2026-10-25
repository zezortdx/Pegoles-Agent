//! Windows named-pipe side of the egress stream.
//!
//! Every handle here is opened for OVERLAPPED I/O: a synchronous pipe
//! handle serializes reads and writes, so a reader blocked on it would
//! stall the writer and deadlock the bridge. Each call therefore runs its
//! own overlapped operation with a private event, which makes one handle
//! safely usable from a reader thread and a writer thread at once, and
//! lets tokio adopt the very same handle (`NamedPipeServer::from_raw_handle`).

use std::io;
use std::os::windows::io::{AsRawHandle, IntoRawHandle, RawHandle};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, LocalFree, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE,
    ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::{is_egress_pipe_name, EGRESS_PIPE_PREFIX};
use crate::error::{ComputerError, Result};

/// Owner only: nobody else (not even SYSTEM or other admins' sessions).
const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)";
const PIPE_BUFFER: u32 = 64 * 1024;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn backend(what: &str, e: impl std::fmt::Display) -> ComputerError {
    ComputerError::Backend(format!("egress endpoint: {what}: {e}"))
}

/// A kernel handle closed once.
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: our event handle, closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// An overlapped named-pipe handle, usable from several threads.
#[derive(Debug)]
pub struct PipeHandle(HANDLE);

// SAFETY: kernel handles may be used from any thread; every I/O call
// carries its own OVERLAPPED and event.
unsafe impl Send for PipeHandle {}
unsafe impl Sync for PipeHandle {}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            // SAFETY: we own this handle and close it once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

impl AsRawHandle for PipeHandle {
    fn as_raw_handle(&self) -> RawHandle {
        self.0 .0
    }
}

impl IntoRawHandle for PipeHandle {
    fn into_raw_handle(self) -> RawHandle {
        let raw = self.0 .0;
        std::mem::forget(self);
        raw
    }
}

fn is_eof(e: &windows::core::Error) -> bool {
    [
        ERROR_BROKEN_PIPE,
        ERROR_HANDLE_EOF,
        ERROR_PIPE_NOT_CONNECTED,
        ERROR_NO_DATA,
    ]
    .iter()
    .any(|c| e.code() == c.to_hresult())
}

impl PipeHandle {
    /// Open the existing pipe `name` as a client (the VM host helper).
    /// Only the exact `pegoles-egress-<uuid>` form is accepted.
    pub fn connect_client(name: &str) -> io::Result<Self> {
        if !is_egress_pipe_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an egress pipe name",
            ));
        }
        let wide_name = wide(name);
        // SAFETY: NUL-terminated name. The anonymous impersonation level
        // means the server cannot act as this process.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
                None,
            )
        }?;
        Ok(Self(handle))
    }

    /// Run one overlapped call to completion (waits; the buffer outlives it).
    fn run(
        &self,
        op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
    ) -> io::Result<usize> {
        // SAFETY: a fresh manual-reset event, owned by `Event`.
        let event = Event(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?);
        let mut overlapped = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        if let Err(e) = op(&mut overlapped) {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return if is_eof(&e) { Ok(0) } else { Err(e.into()) };
            }
        }
        let mut transferred = 0u32;
        // SAFETY: the same handle and OVERLAPPED the call was issued with.
        match unsafe { GetOverlappedResult(self.0, &overlapped, &mut transferred, true) } {
            Ok(()) => Ok(transferred as usize),
            Err(e) if is_eof(&e) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `run` waits for completion before `buf` is released.
        self.run(|ov| unsafe { ReadFile(self.0, Some(buf), None, Some(ov)) })
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: as in `read`.
        self.run(|ov| unsafe { WriteFile(self.0, Some(buf), None, Some(ov)) })
    }

    /// Abort every pending operation on this handle (wakes blocked pumps).
    pub fn cancel(&self) {
        // SAFETY: cancelling I/O on our own handle; failure only means
        // nothing was pending.
        unsafe {
            let _ = CancelIoEx(self.0, None);
        }
    }

    /// A second handle to the same pipe (one per direction).
    pub fn try_clone(&self) -> io::Result<Self> {
        let mut copy = HANDLE::default();
        // SAFETY: duplicating our handle within this process.
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                self.0,
                GetCurrentProcess(),
                &mut copy,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }?;
        Ok(Self(copy))
    }
}

/// A created, listening pipe nobody has connected to yet.
pub(crate) struct PendingEndpoint {
    name: String,
    pipe: PipeHandle,
}

impl PendingEndpoint {
    pub(crate) fn create(_data_root: &std::path::Path) -> Result<Self> {
        let name = format!("{EGRESS_PIPE_PREFIX}{}", uuid::Uuid::new_v4());
        debug_assert!(is_egress_pipe_name(&name));
        let wide_name = wide(&name);
        let sddl = wide(PIPE_SDDL);
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: NUL-terminated SDDL; freed below once the pipe exists.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| backend("security descriptor", e))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: NUL-terminated name, valid attributes; one instance, the
        // first (a squatter's pre-created pipe makes this fail).
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide_name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER,
                PIPE_BUFFER,
                0,
                Some(&attributes),
            )
        };
        // SAFETY: allocated by the conversion call above.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        if handle.is_invalid() {
            return Err(backend(
                "cannot create the pipe",
                io::Error::last_os_error(),
            ));
        }
        Ok(Self {
            name,
            pipe: PipeHandle(handle),
        })
    }

    /// What goes into `egress_open.endpoint`.
    pub(crate) fn wire_endpoint(&self) -> String {
        self.name.clone()
    }

    /// Accept exactly one client within `timeout`, then the pipe has no
    /// further instance (nothing else can connect).
    pub(crate) fn accept(self, timeout: Duration) -> Result<super::EgressEndpoint> {
        // SAFETY: a fresh manual-reset event, owned by `Event`.
        let event = Event(
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
                .map_err(|e| backend("event", e))?,
        );
        let mut overlapped = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        // SAFETY: our pipe and OVERLAPPED, both alive until completion.
        let started = unsafe { ConnectNamedPipe(self.pipe.0, Some(&mut overlapped)) };
        match started {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                // The helper connected before we asked.
                return Ok(super::EgressEndpoint::Pipe(self.pipe));
            }
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(e) => return Err(backend("cannot wait for the helper", e)),
        }
        let deadline = Instant::now() + timeout;
        let wait_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(u128::from(u32::MAX - 1)) as u32;
        // SAFETY: waiting on our event.
        if unsafe { WaitForSingleObject(event.0, wait_ms) } != WAIT_OBJECT_0 {
            self.pipe.cancel();
            let mut ignored = 0u32;
            // SAFETY: finish the cancelled operation before `overlapped` dies.
            unsafe {
                let _ = GetOverlappedResult(self.pipe.0, &overlapped, &mut ignored, true);
            }
            return Err(ComputerError::Timeout(
                "the helper did not connect to the egress endpoint".to_string(),
            ));
        }
        let mut ignored = 0u32;
        // SAFETY: the completed connect on our pipe.
        unsafe { GetOverlappedResult(self.pipe.0, &overlapped, &mut ignored, false) }
            .map_err(|e| backend("connect failed", e))?;
        Ok(super::EgressEndpoint::Pipe(self.pipe))
    }
}

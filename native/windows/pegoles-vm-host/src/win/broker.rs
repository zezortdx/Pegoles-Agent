//! Client of the PegolesVmBroker service: start it on demand, check that
//! the pipe really belongs to Pegoles' broker binary, then speak the
//! typed protocol (`pegoles-broker-proto`).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::time::{Duration, Instant};

use pegoles_broker_proto::{self as proto, Request, Response};
use windows::core::PWSTR;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_SERVICE_ALREADY_RUNNING, GENERIC_READ,
    GENERIC_WRITE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_IMPERSONATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, StartServiceW,
    SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START, SERVICE_STATUS_PROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::util::{pcwstr, wide, Owned};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const BROKER_EXE: &str = "pegoles-broker.exe";

struct Svc(SC_HANDLE);

impl Drop for Svc {
    fn drop(&mut self) {
        // SAFETY: a handle we opened, closed once.
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn running(service: &Svc) -> bool {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    // SAFETY: the buffer is exactly one SERVICE_STATUS_PROCESS.
    let ok = unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            Some(std::slice::from_raw_parts_mut(
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
                std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
            )),
            &mut needed,
        )
    }
    .is_ok();
    ok && status.dwCurrentState == SERVICE_RUNNING
}

/// Start the demand-start service (interactive users may start it; see
/// the service DACL set by the installer) and wait until it runs.
fn ensure_service() -> Result<(), String> {
    let name = wide(proto::SERVICE_NAME);
    // SAFETY: connect-only access to the local service manager.
    let scm = Svc(unsafe {
        OpenSCManagerW(
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            SC_MANAGER_CONNECT,
        )
    }
    .map_err(|e| format!("cannot reach the service manager: {e}"))?);
    // SAFETY: start + query rights only.
    let service =
        Svc(
            unsafe { OpenServiceW(scm.0, pcwstr(&name), SERVICE_START | SERVICE_QUERY_STATUS) }
                .map_err(|e| {
                    format!("pegoles-vm-broker is not installed ({e}); install Pegoles again")
                })?,
        );
    if running(&service) {
        return Ok(());
    }
    // SAFETY: no arguments.
    if let Err(e) = unsafe { StartServiceW(service.0, None) } {
        if e.code() != ERROR_SERVICE_ALREADY_RUNNING.to_hresult() {
            return Err(format!("cannot start pegoles-vm-broker: {e}"));
        }
    }
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if running(&service) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("pegoles-vm-broker did not start in time".into())
}

fn image_of(pid: u32) -> Result<String, String> {
    // SAFETY: limited query access is enough to read an image path.
    let process = Owned(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|e| format!("cannot inspect the broker: {e}"))?,
    );
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: buffer and size describe the same allocation.
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .map_err(|e| format!("cannot inspect the broker: {e}"))?;
    Ok(String::from_utf16_lossy(&buffer[..size as usize]))
}

fn open_pipe() -> Result<Owned, String> {
    let name = wide(proto::PIPE_NAME);
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        // SAFETY: plain open of the broker's pipe. The broker may
        // impersonate us (it opens our files as us); we check who it is
        // before sending anything.
        match unsafe {
            CreateFileW(
                pcwstr(&name),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IMPERSONATION,
                None,
            )
        } {
            Ok(handle) => return Ok(Owned(handle)),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!("cannot reach pegoles-vm-broker: {e}"))
            }
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                // SAFETY: NUL-terminated name.
                unsafe {
                    let _ = WaitNamedPipeW(pcwstr(&name), 2000);
                }
            }
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(format!("cannot reach pegoles-vm-broker: {e}")),
        }
    }
}

pub struct Broker {
    writer: File,
    reader: BufReader<File>,
}

impl Broker {
    pub fn connect() -> Result<Self, String> {
        ensure_service()?;
        let pipe = open_pipe()?;
        let mut pid = 0u32;
        // SAFETY: a connected client pipe handle.
        unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) }
            .map_err(|e| format!("cannot identify the broker: {e}"))?;
        let expected = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name(BROKER_EXE);
        let image = image_of(pid)?;
        if !proto::same_windows_path(&expected.to_string_lossy(), &image) {
            return Err(format!(
                "the broker pipe belongs to {image}, not Pegoles; refusing"
            ));
        }
        let raw = pipe.0;
        std::mem::forget(pipe);
        // SAFETY: we own `raw` now (the Owned wrapper was forgotten).
        let writer = unsafe { File::from_raw_handle(raw.0) };
        let reader = BufReader::new(writer.try_clone().map_err(|e| e.to_string())?);
        let mut broker = Self { writer, reader };
        let hello = broker.call(&Request::Hello {
            version: proto::PROTOCOL_VERSION,
        })?;
        if !hello.ok {
            return Err(hello
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "the broker refused Pegoles".into()));
        }
        Ok(broker)
    }

    pub fn call(&mut self, request: &Request) -> Result<Response, String> {
        let mut line = serde_json::to_string(request).map_err(|e| e.to_string())?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| format!("broker: {e}"))?;
        let mut reply = String::new();
        (&mut self.reader)
            .take(proto::MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut reply)
            .map_err(|e| format!("broker: {e}"))?;
        if reply.is_empty() {
            return Err("the broker closed the connection".into());
        }
        serde_json::from_str(reply.trim_end())
            .map_err(|_| "the broker sent an unreadable reply".into())
    }
}

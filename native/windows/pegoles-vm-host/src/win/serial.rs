//! COM1 of the VM → a pipe this process owns → `logs/serial.log`
//! (bounded). Boot diagnostics only; nothing is ever sent to the guest.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

use super::util::{pcwstr, wide, Owned};

/// SYSTEM, the owner (this user) and Hyper-V's VM worker accounts
/// ("NT VIRTUAL MACHINE\Virtual Machines", S-1-5-83-0) only.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;OW)(A;;GRGW;;;S-1-5-83-0)";
/// Log cap; the engine rotates while the computer is stopped.
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

pub struct SerialPump {
    closed: Arc<AtomicBool>,
    name: Vec<u16>,
}

impl SerialPump {
    pub fn open(computer_id: &str, log_path: &str) -> Result<Self, String> {
        let pipe_name = pegoles_broker_proto::serial_pipe(computer_id);
        let name = wide(&pipe_name);
        let sddl = wide(PIPE_SDDL);
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: NUL-terminated SDDL; the descriptor is leaked for the
        // pipe's lifetime (one small allocation per boot).
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                pcwstr(&sddl),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| format!("serial pipe security: {e}"))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: NUL-terminated name, valid attributes; one instance.
        let pipe = unsafe {
            CreateNamedPipeW(
                pcwstr(&name),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                64 * 1024,
                0,
                Some(&attributes),
            )
        };
        if pipe.is_invalid() {
            return Err(format!("cannot create {pipe_name}"));
        }
        let pipe = Owned(pipe);
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|e| format!("cannot open the serial log: {e}"))?;
        let closed = Arc::new(AtomicBool::new(false));
        let stop = closed.clone();
        std::thread::Builder::new()
            .name("pegoles-serial".into())
            .spawn(move || {
                // SAFETY: our server pipe; blocks until the VM connects.
                if unsafe { ConnectNamedPipe(pipe.raw(), None) }.is_err()
                    && stop.load(Ordering::SeqCst)
                {
                    return;
                }
                let mut written = log.metadata().map(|m| m.len()).unwrap_or(0);
                let mut buffer = [0u8; 4096];
                while !stop.load(Ordering::SeqCst) {
                    let mut read = 0u32;
                    // SAFETY: valid pipe; buffer and length from the same array.
                    if unsafe { ReadFile(pipe.raw(), Some(&mut buffer), Some(&mut read), None) }
                        .is_err()
                        || read == 0
                    {
                        break;
                    }
                    if written < MAX_LOG_BYTES {
                        let _ = log.write_all(&buffer[..read as usize]);
                        written += u64::from(read);
                    }
                }
            })
            .map_err(|e| format!("cannot start the serial log: {e}"))?;
        Ok(Self { closed, name })
    }

    /// Stop logging. Wakes a pipe still waiting for the VM.
    pub fn close(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // SAFETY: a throwaway client connection to our own pipe.
        if let Ok(h) = unsafe {
            CreateFileW(
                pcwstr(&self.name),
                GENERIC_WRITE.0,
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
}

impl Drop for SerialPump {
    fn drop(&mut self) {
        self.close();
    }
}

//! The egress stream on Hyper-V: this process connects (AF_HYPERV) to the
//! guest forwarder's listener (vsock port 4051) and bridges the raw bytes
//! to the named pipe the app created. Transport only: no parsing, no
//! decisions, no logging of payload. Bounded memory (one buffer per
//! direction), both sides close when either does.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pegoles_computer::egress::{win::PipeHandle, EGRESS_VSOCK_PORT};
use pegoles_computer::vmhost_proto::HostEvent;
use windows::Win32::Networking::WinSock::{
    closesocket, recv, send, shutdown, SD_BOTH, SEND_RECV_FLAGS, SOCKET,
};

use super::link::{guid, try_connect, wsa};
use crate::framer::Guid;
use crate::Out;

/// One pump buffer per direction.
const BUFFER_BYTES: usize = 64 * 1024;
/// The guest forwarder may still be starting; a few short retries.
const CONNECT_ATTEMPTS: u32 = 5;
const CONNECT_RETRY: Duration = Duration::from_millis(300);
/// Teardown waits this long for the pumps to notice.
const TEARDOWN_WAIT: Duration = Duration::from_secs(3);

struct Shared {
    computer_id: String,
    out: Arc<Out>,
    socket: SOCKET,
    pipe: PipeHandle,
    stopping: AtomicBool,
    by_app: AtomicBool,
    reason: Mutex<String>,
    pumps_running: AtomicUsize,
}

// SAFETY: a SOCKET is a plain handle value; the pumps use it concurrently
// only through recv/send/shutdown, which Winsock allows.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// Start tearing down: wake both pumps. Descriptors stay open until
    /// both have exited (see `pump_exited`).
    fn begin(&self, reason: &str, by_app: bool) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        self.by_app.store(by_app, Ordering::SeqCst);
        if let Ok(mut r) = self.reason.lock() {
            *r = reason.to_string();
        }
        // SAFETY: shutting down (not closing) wakes the blocked reader.
        unsafe {
            shutdown(self.socket, SD_BOTH);
        }
        self.pipe.cancel();
    }

    fn pump_exited(&self) {
        if self.pumps_running.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        // SAFETY: both pumps are done; the socket is closed exactly once
        // (the pipe handle closes when `Shared` drops).
        unsafe {
            closesocket(self.socket);
        }
        if !self.by_app.load(Ordering::SeqCst) {
            let reason = self
                .reason
                .lock()
                .map(|r| r.clone())
                .unwrap_or_else(|_| "closed".into());
            self.out.line(&HostEvent::EgressClosed {
                computer_id: self.computer_id.clone(),
                reason,
            });
        }
    }
}

pub struct EgressBridge {
    shared: Arc<Shared>,
}

fn socket_to_pipe(shared: &Shared) -> &'static str {
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        // SAFETY: valid socket; the buffer is ours.
        let n = unsafe { recv(shared.socket, &mut buffer, SEND_RECV_FLAGS(0)) };
        if n == 0 {
            return "guest_closed";
        }
        if n < 0 {
            return "read_error";
        }
        let mut written = 0usize;
        while written < n as usize {
            match shared.pipe.write(&buffer[written..n as usize]) {
                Ok(0) | Err(_) => return "write_error",
                Ok(w) => written += w,
            }
        }
    }
}

fn pipe_to_socket(shared: &Shared) -> &'static str {
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        let n = match shared.pipe.read(&mut buffer) {
            Ok(0) => return "app_closed",
            Ok(n) => n,
            Err(_) => return "read_error",
        };
        let mut sent = 0usize;
        while sent < n {
            // SAFETY: valid socket; the slice is ours.
            let w = unsafe { send(shared.socket, &buffer[sent..n], SEND_RECV_FLAGS(0)) };
            if w <= 0 {
                return "write_error";
            }
            sent += w as usize;
        }
    }
}

impl EgressBridge {
    /// Connect the guest first (so a guest that is not listening never
    /// touches the endpoint), then the app's pipe, then start pumping.
    pub fn open(
        computer_id: &str,
        runtime_id: &str,
        endpoint: &str,
        out: Arc<Out>,
    ) -> Result<Self, String> {
        if !pegoles_computer::egress::is_egress_pipe_name(endpoint) {
            return Err("the egress endpoint is not an egress pipe".into());
        }
        wsa()?;
        let vm = Guid::parse(runtime_id)
            .map(guid)
            .ok_or("the VM id is not a GUID")?;
        let service = Guid::parse(&pegoles_computer::hyperv_service_guid_for_port(
            EGRESS_VSOCK_PORT,
        ))
        .map(guid)
        .ok_or("bad egress service GUID")?;
        let mut socket = None;
        for attempt in 0..CONNECT_ATTEMPTS {
            socket = try_connect(vm, service);
            if socket.is_some() {
                break;
            }
            if attempt + 1 < CONNECT_ATTEMPTS {
                std::thread::sleep(CONNECT_RETRY);
            }
        }
        let socket = socket.ok_or("the guest is not listening for the egress stream")?;
        let pipe = match PipeHandle::connect_client(endpoint) {
            Ok(pipe) => pipe,
            Err(e) => {
                // SAFETY: our socket, closed once.
                unsafe {
                    closesocket(socket);
                }
                return Err(format!("cannot connect to the app's endpoint: {e}"));
            }
        };
        let shared = Arc::new(Shared {
            computer_id: computer_id.to_string(),
            out,
            socket,
            pipe,
            stopping: AtomicBool::new(false),
            by_app: AtomicBool::new(false),
            reason: Mutex::new("closed".into()),
            pumps_running: AtomicUsize::new(2),
        });
        type Pump = fn(&Shared) -> &'static str;
        let pumps: [(&str, Pump); 2] = [
            ("pegoles-egress-guest-to-app", socket_to_pipe),
            ("pegoles-egress-app-to-guest", pipe_to_socket),
        ];
        for (name, pump) in pumps {
            let worker = shared.clone();
            let spawned = std::thread::Builder::new()
                .name(name.into())
                .stack_size(256 * 1024)
                .spawn(move || {
                    let reason = pump(&worker);
                    worker.begin(reason, false);
                    worker.pump_exited();
                });
            if let Err(e) = spawned {
                // Stop the pump that did start; account for the one that
                // never will.
                shared.begin("internal", true);
                shared.pump_exited();
                return Err(format!("cannot start the egress pumps: {e}"));
            }
        }
        Ok(Self { shared })
    }

    /// The app asked to close (or the VM is going away). Silent.
    pub fn close(&self) {
        self.shared.begin("closed", true);
        let deadline = Instant::now() + TEARDOWN_WAIT;
        // A pump can race past the first cancel into a fresh blocking
        // call; keep cancelling until both have exited.
        while self.shared.pumps_running.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            self.shared.pipe.cancel();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn is_open(&self) -> bool {
        !self.shared.stopping.load(Ordering::SeqCst)
    }
}

impl Drop for EgressBridge {
    fn drop(&mut self) {
        self.close();
    }
}

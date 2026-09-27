//! The guest channel on Hyper-V: this process connects (AF_HYPERV) to the
//! guest runtime's privileged listener, retrying while the VM boots and
//! after the runtime restarts. Only SYSTEM and the signed-in user may
//! connect to the VM's sockets (HCS HvSocket security descriptors).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use pegoles_computer::vmhost_proto::HostEvent;
use windows::core::GUID;
use windows::Win32::Networking::WinSock::{
    closesocket, connect, recv, send, setsockopt, shutdown, socket, WSAStartup, AF_HYPERV, SD_BOTH,
    SEND_RECV_FLAGS, SOCKADDR, SOCKET, SOCK_STREAM, WSADATA,
};
use windows::Win32::System::Hypervisor::{
    HVSOCKET_CONNECTED_SUSPEND, HVSOCKET_CONNECT_TIMEOUT, HV_PROTOCOL_RAW, SOCKADDR_HV,
};

use crate::framer::{Guid, StreamFramer};
use crate::Out;

const CONNECT_TIMEOUT_MS: u32 = 2000;
const RETRY: Duration = Duration::from_millis(250);

fn guid(g: Guid) -> GUID {
    GUID::from_values(g.data1, g.data2, g.data3, g.data4)
}

fn wsa() -> Result<(), String> {
    static ONCE: Once = Once::new();
    static mut OK: bool = false;
    ONCE.call_once(|| {
        let mut data = WSADATA::default();
        // SAFETY: once per process; the result is read below.
        unsafe { OK = WSAStartup(0x0202, &mut data) == 0 };
    });
    // SAFETY: written once inside call_once before any read.
    if unsafe { OK } {
        Ok(())
    } else {
        Err("Windows sockets are unavailable".into())
    }
}

struct Shared {
    computer_id: String,
    out: Arc<Out>,
    socket: Mutex<Option<SOCKET>>,
    connected: AtomicBool,
    closed: AtomicBool,
    kick_reason: Mutex<Option<String>>,
}

pub struct GuestLink {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn try_connect(vm: GUID, service: GUID) -> Option<SOCKET> {
    // SAFETY: plain socket creation.
    let sock = unsafe { socket(AF_HYPERV as i32, SOCK_STREAM, HV_PROTOCOL_RAW as i32) }.ok()?;
    let timeout = CONNECT_TIMEOUT_MS.to_le_bytes();
    let suspend = 1u32.to_le_bytes();
    // SAFETY: option buffers are the documented 4-byte values.
    unsafe {
        setsockopt(
            sock,
            HV_PROTOCOL_RAW as i32,
            HVSOCKET_CONNECT_TIMEOUT as i32,
            Some(&timeout),
        );
        // Keep the connection across a VM pause instead of dropping it.
        setsockopt(
            sock,
            HV_PROTOCOL_RAW as i32,
            HVSOCKET_CONNECTED_SUSPEND as i32,
            Some(&suspend),
        );
    }
    let address = SOCKADDR_HV {
        Family: windows::Win32::Networking::WinSock::ADDRESS_FAMILY(AF_HYPERV),
        Reserved: 0,
        VmId: vm,
        ServiceId: service,
    };
    // SAFETY: a correctly sized SOCKADDR_HV.
    let rc = unsafe {
        connect(
            sock,
            (&address as *const SOCKADDR_HV).cast::<SOCKADDR>(),
            std::mem::size_of::<SOCKADDR_HV>() as i32,
        )
    };
    if rc == 0 {
        Some(sock)
    } else {
        // SAFETY: our socket, closed once.
        unsafe {
            closesocket(sock);
        }
        None
    }
}

fn pump(shared: &Shared, sock: SOCKET) -> String {
    let mut framer = StreamFramer::new();
    let mut buffer = [0u8; 8192];
    loop {
        // SAFETY: valid socket; the buffer is ours.
        let n = unsafe { recv(sock, &mut buffer, SEND_RECV_FLAGS(0)) };
        if n <= 0 {
            return if n == 0 {
                "eof".into()
            } else {
                "read_error".into()
            };
        }
        match framer.push(&buffer[..n as usize]) {
            Ok(lines) => {
                for payload in lines {
                    shared.out.line(&HostEvent::GuestFrame {
                        computer_id: shared.computer_id.clone(),
                        payload,
                    });
                }
            }
            Err(reason) => return reason.to_string(),
        }
    }
}

impl GuestLink {
    pub fn open(computer_id: &str, runtime_id: &str, out: Arc<Out>) -> Result<Self, String> {
        wsa()?;
        let vm = Guid::parse(runtime_id)
            .map(guid)
            .ok_or("the VM id is not a GUID")?;
        let service = Guid::parse(&pegoles_computer::pegoles_hyperv_service_guid())
            .map(guid)
            .ok_or("bad service GUID")?;
        let shared = Arc::new(Shared {
            computer_id: computer_id.to_string(),
            out,
            socket: Mutex::new(None),
            connected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            kick_reason: Mutex::new(None),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pegoles-guest-link".into())
            .spawn(move || {
                while !worker.closed.load(Ordering::SeqCst) {
                    let Some(sock) = try_connect(vm, service) else {
                        std::thread::sleep(RETRY);
                        continue;
                    };
                    if worker.closed.load(Ordering::SeqCst) {
                        unsafe {
                            closesocket(sock);
                        }
                        break;
                    }
                    *worker.socket.lock().expect("socket") = Some(sock);
                    worker.connected.store(true, Ordering::SeqCst);
                    worker.out.line(&HostEvent::GuestConnected {
                        computer_id: worker.computer_id.clone(),
                    });
                    let mut reason = pump(&worker, sock);
                    if let Some(kicked) = worker.kick_reason.lock().expect("kick").take() {
                        reason = kicked;
                    }
                    worker.connected.store(false, Ordering::SeqCst);
                    if let Some(sock) = worker.socket.lock().expect("socket").take() {
                        // SAFETY: our socket, closed once.
                        unsafe {
                            closesocket(sock);
                        }
                    }
                    worker.out.line(&HostEvent::GuestDisconnected {
                        computer_id: worker.computer_id.clone(),
                        reason,
                    });
                    std::thread::sleep(RETRY);
                }
            })
            .map_err(|e| format!("cannot start the guest link: {e}"))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    pub fn send(&self, payload: &str) -> Result<(), String> {
        let guard = self
            .shared
            .socket
            .lock()
            .map_err(|_| "guest link poisoned")?;
        let Some(sock) = *guard else {
            return Err("no guest connection".into());
        };
        let mut line = payload.as_bytes().to_vec();
        line.push(b'\n');
        let mut sent = 0usize;
        while sent < line.len() {
            // SAFETY: valid socket; the slice is ours.
            let n = unsafe { send(sock, &line[sent..], SEND_RECV_FLAGS(0)) };
            if n <= 0 {
                return Err("the guest connection closed".into());
            }
            sent += n as usize;
        }
        Ok(())
    }

    pub fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }

    /// Drop the current connection; the link reconnects on its own.
    pub fn kick(&self, reason: &str) {
        *self.shared.kick_reason.lock().expect("kick") = Some(reason.to_string());
        if let Some(sock) = *self.shared.socket.lock().expect("socket") {
            // SAFETY: shutting down (not closing) wakes the reader.
            unsafe {
                shutdown(sock, SD_BOTH);
            }
        }
    }

    pub fn close(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.kick("closed");
        if let Some(thread) = self.thread.take() {
            // A connect in progress times out within CONNECT_TIMEOUT_MS.
            let _ = thread.join();
        }
    }
}

impl Drop for GuestLink {
    fn drop(&mut self) {
        self.close();
    }
}

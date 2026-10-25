//! Service Control Manager glue: run the pipe server as the
//! `PegolesVmBroker` service, stop on request or when idle.

use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{ERROR_CALL_NOT_IMPLEMENTED, NO_ERROR};
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
    SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_STATUS,
    SERVICE_STATUS_CURRENT_STATE, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};

use super::server::{self, StopFlag, SESSIONS};
use super::util::{pcwstr, wide};

/// With no helper connected this long, the service stops (it is demand
/// started again by the next helper).
const IDLE_STOP: Duration = Duration::from_secs(10 * 60);

static STOP: OnceLock<StopFlag> = OnceLock::new();
static HANDLE: OnceLock<usize> = OnceLock::new();

fn set_state(state: SERVICE_STATUS_CURRENT_STATE, exit_code: u32) {
    let Some(&raw) = HANDLE.get() else { return };
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: if state == SERVICE_STOP_PENDING {
            30_000
        } else {
            0
        },
    };
    // SAFETY: the handle registered in service_main; a valid status.
    unsafe {
        let _ = SetServiceStatus(SERVICE_STATUS_HANDLE(raw as *mut _), &status);
    }
}

unsafe extern "system" fn handler(
    control: u32,
    _event: u32,
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            set_state(SERVICE_STOP_PENDING, 0);
            if let Some(stop) = STOP.get() {
                stop.stop();
            }
            NO_ERROR.0
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR.0,
        _ => ERROR_CALL_NOT_IMPLEMENTED.0,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let stop = STOP.get_or_init(StopFlag::default).clone();
    let name = wide(pegoles_broker_proto::SERVICE_NAME);
    // SAFETY: NUL-terminated name, static handler, no context.
    let Ok(handle) = (unsafe { RegisterServiceCtrlHandlerExW(pcwstr(&name), Some(handler), None) })
    else {
        return;
    };
    let _ = HANDLE.set(handle.0 as usize);
    set_state(SERVICE_RUNNING, 0);
    // Idle watchdog.
    let watch = stop.clone();
    std::thread::spawn(move || {
        let mut idle_since = Instant::now();
        while !watch.stopped() {
            std::thread::sleep(Duration::from_secs(5));
            if SESSIONS.load(Ordering::SeqCst) > 0 {
                idle_since = Instant::now();
            } else if idle_since.elapsed() >= IDLE_STOP {
                set_state(SERVICE_STOP_PENDING, 0);
                watch.stop();
            }
        }
    });
    let exit = match server::serve(&stop) {
        Ok(()) => 0,
        Err(_) => 1,
    };
    set_state(SERVICE_STOPPED, exit);
}

/// Hand the process to the Service Control Manager (returns when the
/// service has stopped).
pub fn run() -> i32 {
    let mut name = wide(pegoles_broker_proto::SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR::null(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: a NUL-terminated table that outlives the dispatcher call.
    match unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pegoles-broker: not started by the Service Control Manager ({e}); use run-foreground for a console");
            1
        }
    }
}

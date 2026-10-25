//! Live Windows detection and the two consented setup steps. Each call
//! is independent: a failing one leaves its fact unknown, never "ready".

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_SUCCESS, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
};
use windows::Win32::System::Shutdown::{
    InitiateShutdownW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OPERATINGSYSTEM,
    SHTDN_REASON_MINOR_RECONFIG, SHUTDOWN_RESTART,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, IsProcessorFeaturePresent, OpenProcessToken,
    WaitForSingleObject, INFINITE, PF_VIRT_FIRMWARE_ENABLED,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::{readiness_from, EnableOutcome, ReadinessFacts, WindowsReadiness};
use crate::error::{ComputerError, Result};

const NT_CURRENT_VERSION: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// Same name as the product, so the uninstaller's cleanup covers it.
const RUN_VALUE: PCWSTR = w!("Pegoles Agent");
/// DISM: done, completes after a restart.
const RESTART_REQUIRED: u32 = 3010;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_string(hive: HKEY, key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: buffer and byte size describe the same array; REG_SZ only.
    let status = unsafe {
        RegGetValueW(
            hive,
            key,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

fn key_exists(hive: HKEY, key: PCWSTR) -> bool {
    let mut handle = HKEY::default();
    // SAFETY: read-only open; closed right away.
    let status = unsafe { RegOpenKeyExW(hive, key, None, KEY_READ, &mut handle) };
    if status == ERROR_SUCCESS {
        unsafe {
            let _ = RegCloseKey(handle);
        }
        true
    } else {
        false
    }
}

fn service_installed(name: PCWSTR) -> Option<bool> {
    // SAFETY: connect-only access to the local service manager.
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }.ok()?;
    // SAFETY: query-only open; handles closed below.
    let service = unsafe { OpenServiceW(scm, name, SERVICE_QUERY_STATUS) };
    let found = match service {
        Ok(handle) => {
            unsafe {
                let _ = CloseServiceHandle(handle);
            }
            true
        }
        Err(_) => false,
    };
    unsafe {
        let _ = CloseServiceHandle(scm);
    }
    Some(found)
}

#[cfg(target_arch = "x86_64")]
// `__cpuid` became a safe fn in newer toolchains; keep both building.
#[allow(unused_unsafe)]
fn cpu() -> (bool, Option<bool>) {
    use std::arch::x86_64::__cpuid;
    // SAFETY: CPUID is available on every x86_64 processor.
    let leaf1 = unsafe { __cpuid(1) };
    let hypervisor = leaf1.ecx & (1 << 31) != 0;
    let vmx = leaf1.ecx & (1 << 5) != 0;
    // SAFETY: as above; extended leaves exist on all x86_64 CPUs.
    let max_ext = unsafe { __cpuid(0x8000_0000) }.eax;
    let svm = max_ext >= 0x8000_0001 && unsafe { __cpuid(0x8000_0001) }.ecx & (1 << 2) != 0;
    // Under a hypervisor the guest's CPUID may hide VMX/SVM: unknown then.
    let supports = if hypervisor && !(vmx || svm) {
        None
    } else {
        Some(vmx || svm)
    };
    (hypervisor, supports)
}

#[cfg(not(target_arch = "x86_64"))]
fn cpu() -> (bool, Option<bool>) {
    (false, None)
}

pub(super) fn readiness() -> WindowsReadiness {
    let build = reg_string(
        HKEY_LOCAL_MACHINE,
        NT_CURRENT_VERSION,
        w!("CurrentBuildNumber"),
    )
    .and_then(|b| b.trim().parse::<u32>().ok());
    let edition = reg_string(HKEY_LOCAL_MACHINE, NT_CURRENT_VERSION, w!("EditionID"));
    let display = reg_string(HKEY_LOCAL_MACHINE, NT_CURRENT_VERSION, w!("DisplayVersion"));
    let (hypervisor_present, cpu_supports_vt) = cpu();
    // SAFETY: plain feature query.
    let firmware = unsafe { IsProcessorFeaturePresent(PF_VIRT_FIRMWARE_ENABLED) }.as_bool();
    let reboot_pending = key_exists(
        HKEY_LOCAL_MACHINE,
        w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"),
    ) || key_exists(
        HKEY_LOCAL_MACHINE,
        w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired"),
    );
    let broker = wide(pegoles_broker_name());
    let facts = ReadinessFacts {
        build,
        x64: cfg!(target_arch = "x86_64"),
        hypervisor_present,
        cpu_supports_vt,
        // Meaningless while a hypervisor runs (it hides the flag).
        firmware_vt: (!hypervisor_present).then_some(firmware),
        vmcompute_installed: service_installed(w!("vmcompute")),
        reboot_pending: Some(reboot_pending),
        broker_installed: service_installed(PCWSTR(broker.as_ptr())),
    };
    readiness_from(&facts, edition.as_deref(), display.as_deref())
}

/// The broker's service name (kept in one place: pegoles-broker-proto).
fn pegoles_broker_name() -> &'static str {
    "PegolesVmBroker"
}

pub(super) fn enable_virtualization_elevated() -> Result<EnableOutcome> {
    let exe = std::env::current_exe().map_err(|e| ComputerError::Backend(e.to_string()))?;
    let broker = exe
        .parent()
        .map(|dir| dir.join("pegoles-broker.exe"))
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            ComputerError::SetupRequired(
                "pegoles-broker.exe is missing; install Pegoles again".into(),
            )
        })?;
    let file = wide(&broker.to_string_lossy());
    let args = wide("enable-virtualization");
    let verb = wide("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: COM for the shell on this thread, released below.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    // SAFETY: fully initialized SHELLEXECUTEINFOW; strings outlive the call.
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if com.is_ok() {
        unsafe { CoUninitialize() };
    }
    if let Err(e) = launched {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(EnableOutcome::Declined);
        }
        return Err(ComputerError::Backend(format!(
            "could not ask Windows for permission: {e}"
        )));
    }
    let process = info.hProcess;
    if process.is_invalid() {
        return Err(ComputerError::Backend(
            "the setup step did not start".into(),
        ));
    }
    // SAFETY: a process handle we own; waited on and closed once.
    unsafe { WaitForSingleObject(process, INFINITE) };
    let mut code = 1u32;
    let got = unsafe { GetExitCodeProcess(process, &mut code) };
    unsafe {
        let _ = CloseHandle(process);
    }
    got.map_err(|e| ComputerError::Backend(e.to_string()))?;
    match code {
        0 => Ok(EnableOutcome::Enabled),
        RESTART_REQUIRED => Ok(EnableOutcome::RestartRequired),
        other => Err(ComputerError::Backend(format!(
            "Windows could not turn on Virtual Machine Platform (setup exited with {other})"
        ))),
    }
}

fn enable_shutdown_privilege() -> Result<()> {
    let mut token = HANDLE::default();
    // SAFETY: our own process token, adjust + query rights.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    }
    .map_err(|e| ComputerError::Backend(e.to_string()))?;
    let mut luid = LUID::default();
    // SAFETY: well-known privilege name.
    let looked = unsafe { LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid) };
    let adjusted = looked.and_then(|()| {
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        // SAFETY: a single, fully initialized privilege entry.
        unsafe { AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None) }
    });
    unsafe {
        let _ = CloseHandle(token);
    }
    adjusted.map_err(|e| ComputerError::Backend(format!("cannot restart: {e}")))
}

pub(super) fn restart_for_setup() -> Result<()> {
    // Open Pegoles once after the person signs back in (standard users'
    // RunOnce entries don't run, Run ones do; removed on next launch).
    let exe = std::env::current_exe().map_err(|e| ComputerError::Backend(e.to_string()))?;
    let command = wide(&format!("\"{}\"", exe.to_string_lossy()));
    // SAFETY: REG_SZ data with its byte length (including the NUL).
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        );
    }
    enable_shutdown_privilege()?;
    let message = wide("Restarting to finish setting up Pegoles.");
    // SAFETY: local machine, NUL-terminated message, planned reconfiguration.
    let status = unsafe {
        InitiateShutdownW(
            PCWSTR::null(),
            PCWSTR(message.as_ptr()),
            0,
            SHUTDOWN_RESTART,
            SHTDN_REASON_MAJOR_OPERATINGSYSTEM
                | SHTDN_REASON_MINOR_RECONFIG
                | SHTDN_REASON_FLAG_PLANNED,
        )
    };
    if status != ERROR_SUCCESS.0 {
        clear_resume_after_restart();
        return Err(ComputerError::Backend(format!(
            "Windows did not restart (error {status})"
        )));
    }
    Ok(())
}

pub(super) fn clear_resume_after_restart() {
    // SAFETY: deleting our own value; absent is fine.
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
    }
}

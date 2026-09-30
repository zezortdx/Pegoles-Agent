//! One-shot elevated verbs: register the service (installer) and turn on
//! the Virtual Machine Platform (onboarding, after consent).

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SERVICE_EXISTS;
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
use windows::Win32::Storage::FileSystem::DELETE;
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
    CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW, SetServiceObjectSecurity,
    ENUM_SERVICE_TYPE, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE,
    SERVICE_ALL_ACCESS, SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTROL_STOP, SERVICE_DEMAND_START,
    SERVICE_DESCRIPTIONW, SERVICE_ERROR, SERVICE_ERROR_NORMAL, SERVICE_NO_CHANGE,
    SERVICE_QUERY_STATUS, SERVICE_START_TYPE, SERVICE_STATUS, SERVICE_STOP,
    SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::util::{local_free, pcwstr, wide};

/// Service DACL: SYSTEM and administrators manage it; interactive users
/// may query and START it (RP) — never stop, reconfigure or delete it.
const SERVICE_SDDL: &str =
    "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPLOCRRC;;;IU)";
const DESCRIPTION: &str = "Starts and stops Pegoles' isolated computer (a virtual machine with no network) for the signed-in user. Serves only Pegoles; stops itself when idle.";

struct Service(SC_HANDLE);

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: a handle we opened, closed once.
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn own_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    Ok(exe.to_string_lossy().into_owned())
}

fn set_dacl(service: &Service) -> Result<(), String> {
    let sddl = wide(SERVICE_SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: NUL-terminated SDDL; freed below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            pcwstr(&sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| format!("service security: {e}"))?;
    // SAFETY: a valid descriptor for a service we opened with WRITE_DAC.
    let result = unsafe { SetServiceObjectSecurity(service.0, DACL_SECURITY_INFORMATION, sd) };
    unsafe { local_free(sd.0) };
    result.map_err(|e| format!("service security: {e}"))
}

/// Register (or update) the broker service to run this binary.
pub fn install_service() -> Result<(), String> {
    super::util::require_admin_only_folder()?;
    let command = format!("\"{}\" service", own_path()?);
    let name = wide(pegoles_broker_proto::SERVICE_NAME);
    let display = wide(pegoles_broker_proto::SERVICE_DISPLAY_NAME);
    let command_w = wide(&command);
    // SAFETY: local SCM, create rights (the installer runs elevated).
    let scm = Service(
        unsafe {
            OpenSCManagerW(
                PCWSTR::null(),
                PCWSTR::null(),
                SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE,
            )
        }
        .map_err(|e| format!("cannot open the service manager (run elevated): {e}"))?,
    );
    // SAFETY: NUL-terminated strings; LocalSystem (no account/password).
    let created = unsafe {
        CreateServiceW(
            scm.0,
            pcwstr(&name),
            pcwstr(&display),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            pcwstr(&command_w),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    };
    let service = match created {
        Ok(handle) => Service(handle),
        Err(e) if e.code() == ERROR_SERVICE_EXISTS.to_hresult() => {
            // SAFETY: open the existing service to update it in place.
            let handle = unsafe { OpenServiceW(scm.0, pcwstr(&name), SERVICE_ALL_ACCESS) }
                .map_err(|e| format!("cannot open the existing service: {e}"))?;
            let service = Service(handle);
            // SAFETY: point it at this binary; everything else unchanged.
            unsafe {
                ChangeServiceConfigW(
                    service.0,
                    ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                    SERVICE_START_TYPE(SERVICE_NO_CHANGE),
                    SERVICE_ERROR(SERVICE_NO_CHANGE),
                    pcwstr(&command_w),
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                )
            }
            .map_err(|e| format!("cannot update the service: {e}"))?;
            service
        }
        Err(e) => return Err(format!("cannot register the service: {e}")),
    };
    let mut description = wide(DESCRIPTION);
    let info = SERVICE_DESCRIPTIONW {
        lpDescription: PWSTR(description.as_mut_ptr()),
    };
    // SAFETY: a valid description struct for this info level.
    unsafe {
        let _ = ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            Some((&info as *const SERVICE_DESCRIPTIONW).cast()),
        );
    }
    set_dacl(&service)
}

/// Stop and remove the broker service (uninstaller).
pub fn uninstall_service() -> Result<(), String> {
    let name = wide(pegoles_broker_proto::SERVICE_NAME);
    // SAFETY: local SCM, connect rights.
    let scm = Service(
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(|e| format!("cannot open the service manager: {e}"))?,
    );
    // SAFETY: NUL-terminated name.
    let Ok(handle) = (unsafe {
        OpenServiceW(
            scm.0,
            pcwstr(&name),
            SERVICE_STOP | SERVICE_QUERY_STATUS | DELETE.0,
        )
    }) else {
        return Ok(()); // not installed
    };
    let service = Service(handle);
    let mut status = SERVICE_STATUS::default();
    // SAFETY: stop request; failure (already stopped) is fine.
    unsafe {
        let _ = ControlService(service.0, SERVICE_CONTROL_STOP, &mut status);
    }
    // SAFETY: delete (takes effect once all handles close).
    unsafe { DeleteService(service.0) }.map_err(|e| format!("cannot remove the service: {e}"))
}

/// DISM exit code for "done; a restart completes it".
pub const RESTART_REQUIRED: i32 = 3010;

/// Turn on VirtualMachinePlatform with the system's own DISM (fixed
/// arguments, absolute path from GetSystemDirectory — never PATH).
pub fn enable_virtualization() -> i32 {
    let mut buffer = [0u16; 260];
    // SAFETY: buffer and length from the same array.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if len == 0 || len >= buffer.len() {
        eprintln!("pegoles-broker: cannot find the system folder");
        return 1;
    }
    let dism = format!("{}\\dism.exe", String::from_utf16_lossy(&buffer[..len]));
    let status = std::process::Command::new(&dism)
        .args([
            "/Online",
            "/Enable-Feature",
            "/FeatureName:VirtualMachinePlatform",
            "/All",
            "/NoRestart",
            "/Quiet",
        ])
        .status();
    match status.map(|s| s.code()) {
        Ok(Some(0)) => 0,
        Ok(Some(RESTART_REQUIRED)) => RESTART_REQUIRED,
        Ok(code) => {
            eprintln!("pegoles-broker: DISM failed ({code:?})");
            1
        }
        Err(e) => {
            eprintln!("pegoles-broker: cannot run DISM: {e}");
            1
        }
    }
}

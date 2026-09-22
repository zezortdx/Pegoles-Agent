//! pegoles-windows-setup: one-time PRIVILEGED setup utility (Windows).
//!
//! Narrow command set, auditable, never arbitrary:
//! - `check`   — probe host capabilities, print human summary, exit 0/2.
//! - `explain` — print exactly what `register` would modify, change nothing.
//! - `register`— create the Pegoles Hyper-V socket service registry key,
//!   then read it back to verify. Refuses without elevation (capability
//!   check: tries the write, maps ACCESS_DENIED to "re-run elevated").
//!
//! The normal Pegoles runtime NEVER writes privileged state; it only
//! verifies registration via the capability probe. Never run the whole
//! app elevated.

use pegoles_computer::{probe_windows_host, WindowsSupport};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("--version") | Some("-V") => {
            println!("pegoles-windows-setup 0.1.0");
        }
        Some("check") => std::process::exit(cmd_check()),
        Some("explain") => {
            explain();
        }
        Some("register") => std::process::exit(cmd_register()),
        _ => {
            eprintln!("usage: pegoles-windows-setup <check|explain|register|--version>");
            eprintln!("  check    - probe host capabilities, exit 0 when Supported");
            eprintln!("  explain  - show what register would change (no writes)");
            eprintln!("  register - one-time privileged socket service registration");
            std::process::exit(2);
        }
    }
}

fn cmd_check() -> i32 {
    match probe_windows_host() {
        Ok(caps) => {
            println!("platform: windows {}", caps.architecture.as_str());
            println!("state: {}", support_label(caps.state));
            if let Some(v) = &caps.windows_version {
                println!("windows: {v}");
            }
            if let Some(e) = &caps.edition {
                println!("edition: {e}");
            }
            for step in caps.setup_steps() {
                println!("setup: {step}");
            }
            for note in &caps.notes {
                println!("note: {note}");
            }
            if caps.state == WindowsSupport::Supported {
                0
            } else {
                2
            }
        }
        Err(e) => {
            eprintln!("probe failed: {e}");
            2
        }
    }
}

fn support_label(state: WindowsSupport) -> &'static str {
    match state {
        WindowsSupport::Supported => "supported",
        WindowsSupport::SetupRequired => "setup-required",
        WindowsSupport::UnsupportedEdition => "unsupported-edition",
        WindowsSupport::VirtualizationDisabled => "virtualization-disabled",
        WindowsSupport::HyperVDisabled => "hyperv-disabled",
        WindowsSupport::RebootRequired => "reboot-required",
        WindowsSupport::PermissionMissing => "permission-missing",
        WindowsSupport::HcsUnavailable => "hcs-unavailable",
        WindowsSupport::HvSocketRegistrationMissing => "hv-socket-registration-missing",
        WindowsSupport::UnsupportedArchitecture => "unsupported-architecture",
    }
}

fn explain() {
    println!("pegoles-windows-setup register would:");
    println!(
        "  1. create registry key HKLM\\...\\GuestCommunicationServices\\{}",
        pegoles_computer::pegoles_hyperv_service_guid()
    );
    println!("  2. set ElementName = \"Pegoles Guest Control Plane\"");
    println!("  3. read the key back and verify both values");
    println!("  4. change NOTHING else (no features, no groups, no reboot)");
    println!("requires: elevated administrator console on Windows 11 Pro x86_64");
}

fn cmd_register() -> i32 {
    #[cfg(windows)]
    {
        register_windows()
    }
    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("register runs on Windows only (elevated administrator console)");
        2
    }
}

#[cfg(windows)]
fn register_windows() -> i32 {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe {
        let subkey = format!(
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Virtualization\\GuestCommunicationServices\\{}",
            pegoles_computer::pegoles_hyperv_service_guid()
        );
        let subkey_w = wide(&subkey);
        let mut key = HKEY::default();
        let mut disp = 0u32;
        match RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey_w.as_ptr()),
            None,
            None,
            Default::default(),
            KEY_WRITE,
            None,
            &mut key,
            Some(&mut disp),
        ) {
            Ok(_) => {}
            Err(e) if e.code() == ERROR_ACCESS_DENIED => {
                eprintln!(
                    "ACCESS DENIED: re-run this command from an elevated administrator console."
                );
                eprintln!("Pegoles itself never runs elevated; only this one-time setup does.");
                return 2;
            }
            Err(e) => {
                eprintln!("cannot create registry key: {e}");
                return 2;
            }
        }
        let name = wide("ElementName");
        let value = wide("Pegoles Guest Control Plane");
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                value.as_ptr() as *const u8,
                value.len() * std::mem::size_of::<u16>(),
            )
        };
        let written = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)).is_ok();
        let _ = RegCloseKey(key);
        if !written {
            eprintln!("failed to write ElementName value");
            return 2;
        }
        // Verify: read back through the shared probe path.
        match pegoles_computer::probe_windows_host() {
            Ok(caps) if caps.hv_socket_registered == Some(true) => {
                println!(
                    "registered + verified: {}",
                    pegoles_computer::pegoles_hyperv_service_guid()
                );
                0
            }
            Ok(caps) => {
                eprintln!(
                    "write succeeded but verification reads back {:?}; state={:?}",
                    caps.hv_socket_registered, caps.state
                );
                2
            }
            Err(e) => {
                eprintln!("write succeeded but probe failed: {e}");
                2
            }
        }
    }
}

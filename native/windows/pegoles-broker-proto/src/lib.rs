//! The Pegoles VM broker's protocol: the only privileged surface Pegoles
//! adds to Windows.
//!
//! The broker is a demand-start LocalSystem service because the Host
//! Compute System API only serves administrators. Everything it accepts
//! is here, pure and tested on every OS:
//!
//! - one JSON object per line, at most [`MAX_LINE_BYTES`];
//! - a closed set of verbs ([`Request`]), unknown fields refused;
//! - strict value checks ([`CreateSpec::validate`]);
//! - the exact files a computer may use ([`computer_file`]);
//! - the HCS document built from validated values only ([`hcs_document`]).
//!
//! Nothing here runs anything in a guest, reads or writes other files,
//! opens a network or changes Windows settings. See
//! docs/WINDOWS_ARCHITECTURE.md.

use serde::{Deserialize, Serialize};

/// The broker's pipe (local only; remote clients are rejected).
pub const PIPE_NAME: &str = r"\\.\pipe\pegoles-vm-broker";
/// Windows service name.
pub const SERVICE_NAME: &str = "PegolesVmBroker";
pub const SERVICE_DISPLAY_NAME: &str = "Pegoles VM Broker";
/// The only client the broker serves, next to the broker in its
/// (administrator-only) install folder.
pub const CLIENT_EXE: &str = "pegoles-vm-host.exe";
pub const PROTOCOL_VERSION: u32 = 1;
/// Longest request or response line.
pub const MAX_LINE_BYTES: usize = 16 * 1024;
/// The guest's screen (its compositor's output mode, as on macOS).
pub const GUEST_SCREEN: (u32, u32) = (1440, 900);
/// Compute systems one user may have at once.
pub const MAX_COMPUTERS_PER_USER: usize = 2;
pub const MIN_VCPUS: u8 = 1;
pub const MAX_VCPUS: u8 = 8;
pub const MIN_MEMORY_MB: u32 = 1024;
pub const MAX_MEMORY_MB: u32 = 8192;
/// The files of one computer, in `<LocalAppData>\Pegoles\computers\<id>\`.
pub const DISK_FILE: &str = "disk.vhdx";

/// Requests, one per line. Externally tagged by `op`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {
        version: u32,
    },
    Create(CreateSpec),
    Start {
        computer_id: String,
    },
    Pause {
        computer_id: String,
    },
    Resume {
        computer_id: String,
    },
    /// Graceful stop (guest shutdown integration), terminate on failure.
    Shutdown {
        computer_id: String,
    },
    Terminate {
        computer_id: String,
    },
    State {
        computer_id: String,
    },
}

/// What a computer is made of. Every field is checked before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSpec {
    pub computer_id: String,
    /// Must be `<LocalAppData>\Pegoles\computers\<computer_id>\disk.vhdx`
    /// of the connected user.
    pub disk: String,
    pub vcpus: u8,
    pub memory_mb: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker_version: Option<String>,
    /// HCS RuntimeId of the compute system: the AF_HYPERV VmId.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// `created` | `running` | `paused` | `stopped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// `bad_request` | `forbidden` | `limit` | `unknown_computer` |
    /// `hcs` | `internal`.
    pub code: String,
    pub message: String,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    pub fn fail(code: &str, message: impl Into<String>) -> Self {
        let mut message: String = message.into();
        truncate_utf8(&mut message, 512);
        Self {
            ok: false,
            error: Some(ErrorBody {
                code: code.to_string(),
                message,
            }),
            ..Self::default()
        }
    }
}

fn truncate_utf8(text: &mut String, max: usize) {
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
}

/// Parse one request line. Anything but a single, bounded, well-formed
/// request is refused (never partially honoured).
pub fn parse_request(line: &str) -> Result<Request, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err("request too long".into());
    }
    if line.contains('\n') || line.contains('\r') {
        return Err("one request per line".into());
    }
    serde_json::from_str(line).map_err(|_| "malformed request".into())
}

pub fn encode_response(response: &Response) -> String {
    let mut line = serde_json::to_string(response).unwrap_or_else(|_| {
        r#"{"ok":false,"error":{"code":"internal","message":"encode"}}"#.into()
    });
    line.push('\n');
    line
}

/// Canonical lowercase UUID (`8-4-4-4-12` hex): computer ids and HCS ids.
pub fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// A user SID Pegoles serves: local/domain (`S-1-5-21-…`) or Entra ID
/// (`S-1-12-1-…`) accounts. Never well-known or service SIDs.
pub fn is_user_sid(value: &str) -> bool {
    let rest = match value
        .strip_prefix("S-1-5-21-")
        .or_else(|| value.strip_prefix("S-1-12-1-"))
    {
        Some(rest) => rest,
        None => return false,
    };
    let parts: Vec<&str> = rest.split('-').collect();
    (2..=14).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
}

impl CreateSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !is_uuid(&self.computer_id) {
            return Err("computer_id must be a lowercase UUID".into());
        }
        if !(MIN_VCPUS..=MAX_VCPUS).contains(&self.vcpus) {
            return Err(format!("vcpus must be {MIN_VCPUS}–{MAX_VCPUS}"));
        }
        if !(MIN_MEMORY_MB..=MAX_MEMORY_MB).contains(&self.memory_mb)
            || !self.memory_mb.is_multiple_of(2)
        {
            return Err(format!(
                "memory_mb must be an even number {MIN_MEMORY_MB}–{MAX_MEMORY_MB}"
            ));
        }
        Ok(())
    }
}

/// `<local_app_data>\Pegoles\computers\<id>\<file>`: the only place a
/// computer's files may be. `local_app_data` comes from the connected
/// user's own token, never from the request.
pub fn computer_file(local_app_data: &str, computer_id: &str, file: &str) -> String {
    let root = local_app_data.trim_end_matches(['\\', '/']);
    format!(r"{root}\Pegoles\computers\{computer_id}\{file}")
}

/// Windows path equality for the shapes above: case-insensitive, `/`
/// taken as `\`, no `.`/`..` components, no alternate data streams or
/// device prefixes. Anything unusual is simply "not equal".
pub fn same_windows_path(expected: &str, given: &str) -> bool {
    fn normal(path: &str) -> Option<String> {
        let path = path.replace('/', "\\");
        if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") || path.starts_with(r"\\") {
            return None;
        }
        let mut chars = path.chars();
        let drive = chars.next()?;
        if !drive.is_ascii_alphabetic() || chars.next()? != ':' {
            return None;
        }
        let rest = &path[2..];
        if rest.contains(':') || rest.split('\\').any(|c| c == "." || c == "..") {
            return None;
        }
        if rest.contains("\\\\") {
            return None;
        }
        Some(path.to_lowercase())
    }
    matches!((normal(expected), normal(given)), (Some(a), Some(b)) if a == b)
}

/// The COM1 pipe for a computer (the helper creates it; the VM writes the
/// guest's serial console into it).
pub fn serial_pipe(computer_id: &str) -> String {
    format!(r"\\.\pipe\pegoles-serial-{computer_id}")
}

/// HvSocket access: SYSTEM and the signed-in user only (no registry
/// registration is involved for HCS compute systems).
pub fn hvsocket_sddl(user_sid: &str) -> String {
    format!("D:P(A;;FA;;;SY)(A;;FA;;;{user_sid})")
}

/// The HCS compute-system document. Built only from validated values:
/// UEFI boot from the computer's own VHDX (removable-path boot loader,
/// no NVRAM to keep), COM1 to the helper's pipe, HvSocket limited to
/// SYSTEM and the user, and — deliberately — no network adapter, no
/// shared folders (Plan 9 / SMB), no keyboard or mouse. The one display
/// device is Hyper-V's synthetic video: the guest's own screen (its
/// compositor needs a DRM device, and Debian's kernel has no virtual
/// one), which Pegoles never shows on the host and which carries no
/// input; screenshots and input travel the guest channel as on macOS.
/// `ShouldTerminateOnLastHandleClosed` ties the VM to the broker.
pub fn hcs_document(spec: &CreateSpec, disk: &str, user_sid: &str) -> String {
    let sddl = hvsocket_sddl(user_sid);
    serde_json::json!({
        "Owner": "Pegoles",
        "SchemaVersion": { "Major": 2, "Minor": 1 },
        "ShouldTerminateOnLastHandleClosed": true,
        "VirtualMachine": {
            "StopOnReset": true,
            "Chipset": {
                "UseUtc": true,
                "Uefi": {
                    "Console": "ComPort1",
                    "BootThis": { "DevicePath": "Primary disk", "DiskNumber": 0, "DeviceType": "ScsiDrive" }
                }
            },
            "ComputeTopology": {
                "Memory": { "Backing": "Virtual", "SizeInMB": spec.memory_mb },
                "Processor": { "Count": spec.vcpus }
            },
            "Devices": {
                "Scsi": {
                    "Primary disk": {
                        "Attachments": { "0": { "Type": "VirtualDisk", "Path": disk, "ReadOnly": false } }
                    }
                },
                "ComPorts": { "0": { "NamedPipe": serial_pipe(&spec.computer_id) } },
                "VideoMonitor": {
                    "HorizontalResolution": GUEST_SCREEN.0,
                    "VerticalResolution": GUEST_SCREEN.1
                },
                "HvSocket": {
                    "HvSocketConfig": {
                        "DefaultBindSecurityDescriptor": sddl,
                        "DefaultConnectSecurityDescriptor": sddl
                    }
                }
            }
        }
    })
    .to_string()
}

/// HCS compute-system state names → ours (`None` for anything else).
pub fn state_word(hcs_state: &str) -> Option<&'static str> {
    match hcs_state.to_ascii_lowercase().as_str() {
        "created" => Some("created"),
        "running" => Some("running"),
        "paused" => Some("paused"),
        "stopped" => Some("stopped"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
    const SID: &str = "S-1-5-21-3623811015-3361044348-30300820-1013";

    fn spec() -> CreateSpec {
        CreateSpec {
            computer_id: ID.into(),
            disk: computer_file(r"C:\Users\Júlio\AppData\Local", ID, DISK_FILE),
            vcpus: 2,
            memory_mb: 2048,
        }
    }

    #[test]
    fn requests_are_a_closed_set_with_no_extra_fields() {
        let create = format!(
            r#"{{"op":"create","computer_id":"{ID}","disk":"C:\\x\\disk.vhdx","vcpus":2,"memory_mb":2048}}"#
        );
        assert!(matches!(parse_request(&create), Ok(Request::Create(_))));
        assert_eq!(
            parse_request(&format!(r#"{{"op":"start","computer_id":"{ID}"}}"#)),
            Ok(Request::Start {
                computer_id: ID.into()
            })
        );
        for bad in [
            r#"{"op":"exec","command":"cmd.exe"}"#,
            r#"{"op":"start","computer_id":"x","extra":1}"#,
            r#"{"op":"create","computer_id":"x","disk":"d","vcpus":2,"memory_mb":2048,"network":true}"#,
            "not json",
            "",
        ] {
            assert!(parse_request(bad).is_err(), "{bad}");
        }
        assert!(parse_request(&"x".repeat(MAX_LINE_BYTES + 1)).is_err());
        assert!(parse_request("{\"op\":\"hello\",\"version\":1}\n{}").is_err());
    }

    #[test]
    fn values_are_bounded() {
        assert!(spec().validate().is_ok());
        for bad in [
            CreateSpec {
                computer_id: ID.to_uppercase(),
                ..spec()
            },
            CreateSpec {
                computer_id: "../../etc".into(),
                ..spec()
            },
            CreateSpec { vcpus: 0, ..spec() },
            CreateSpec { vcpus: 9, ..spec() },
            CreateSpec {
                memory_mb: 512,
                ..spec()
            },
            CreateSpec {
                memory_mb: 2049,
                ..spec()
            },
            CreateSpec {
                memory_mb: 65536,
                ..spec()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_real_user_sids() {
        assert!(is_user_sid(SID));
        assert!(is_user_sid(
            "S-1-12-1-1234567890-1234567890-1234567890-1234567890"
        ));
        for bad in [
            "S-1-5-18",
            "S-1-1-0",
            "S-1-5-32-544",
            "S-1-5-21",
            "S-1-5-21-1-2-3)(A;;FA;;;WD",
            "S-1-5-21-12345678901-1",
            "",
        ] {
            assert!(!is_user_sid(bad), "{bad}");
        }
    }

    #[test]
    fn computer_files_live_in_the_users_own_folder() {
        let root = r"C:\Users\Júlio Cabral\AppData\Local";
        let disk = computer_file(root, ID, DISK_FILE);
        assert_eq!(
            disk,
            format!(r"C:\Users\Júlio Cabral\AppData\Local\Pegoles\computers\{ID}\disk.vhdx")
        );
        assert!(same_windows_path(
            &disk,
            &disk.to_uppercase().replace("JÚLIO", "Júlio")
        ));
        assert!(same_windows_path(&disk, &disk.replace('\\', "/")));
        for bad in [
            format!(
                r"C:\Users\Júlio Cabral\AppData\Local\Pegoles\computers\{ID}\..\..\x\disk.vhdx"
            ),
            format!(r"\\?\C:\Users\Júlio Cabral\AppData\Local\Pegoles\computers\{ID}\disk.vhdx"),
            format!(r"\\server\share\Pegoles\computers\{ID}\disk.vhdx"),
            format!(r"C:\Users\Júlio Cabral\AppData\Local\Pegoles\computers\{ID}\disk.vhdx:ads"),
            format!(r"C:\Users\Other\AppData\Local\Pegoles\computers\{ID}\disk.vhdx"),
        ] {
            assert!(!same_windows_path(&disk, &bad), "{bad}");
        }
    }

    #[test]
    fn the_hcs_document_has_no_network_no_shares_no_input_and_a_locked_socket() {
        let s = spec();
        let doc: serde_json::Value = serde_json::from_str(&hcs_document(&s, &s.disk, SID)).unwrap();
        let vm = &doc["VirtualMachine"];
        let devices = vm["Devices"].as_object().unwrap();
        let mut names: Vec<&String> = devices.keys().collect();
        names.sort();
        assert_eq!(names, ["ComPorts", "HvSocket", "Scsi", "VideoMonitor"]);
        assert_eq!(vm["Devices"]["VideoMonitor"]["HorizontalResolution"], 1440);
        assert!(vm["Devices"].get("Keyboard").is_none() && vm["Devices"].get("Mouse").is_none());
        assert!(vm.get("GuestState").is_none());
        assert_eq!(doc["ShouldTerminateOnLastHandleClosed"], true);
        assert_eq!(vm["ComputeTopology"]["Memory"]["SizeInMB"], 2048);
        assert_eq!(vm["ComputeTopology"]["Processor"]["Count"], 2);
        assert_eq!(
            vm["Devices"]["Scsi"]["Primary disk"]["Attachments"]["0"]["Path"],
            s.disk
        );
        assert_eq!(
            vm["Devices"]["ComPorts"]["0"]["NamedPipe"],
            format!(r"\\.\pipe\pegoles-serial-{ID}")
        );
        let sddl = format!("D:P(A;;FA;;;SY)(A;;FA;;;{SID})");
        let hv = &vm["Devices"]["HvSocket"]["HvSocketConfig"];
        assert_eq!(hv["DefaultBindSecurityDescriptor"], sddl);
        assert_eq!(hv["DefaultConnectSecurityDescriptor"], sddl);
        assert_eq!(vm["Chipset"]["Uefi"]["BootThis"]["DeviceType"], "ScsiDrive");
    }

    #[test]
    fn responses_are_one_bounded_line() {
        let long = Response::fail("hcs", "é".repeat(600));
        let line = encode_response(&long);
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        assert!(long.error.unwrap().message.len() <= 512);
        assert_eq!(state_word("Running"), Some("running"));
        assert_eq!(state_word("SavedAsTemplate"), None);
    }
}

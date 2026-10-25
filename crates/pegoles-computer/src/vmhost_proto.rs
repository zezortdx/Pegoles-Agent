//! Rust side of the Rust <-> `pegoles-vm-host` IPC protocol.
//!
//! Transport: JSON Lines over the child process stdin/stdout (one JSON
//! object per line). The full contract is documented in
//! `docs/VM_HOST_PROTOCOL.md`.
//!
//! SECURITY: the command set is closed. There is no shell execution, no
//! host filesystem operation, and no arbitrary native call. Unknown
//! commands are rejected by the Swift side and unknown lines are rejected
//! here. The model can never reach this channel.

use serde::{Deserialize, Serialize};

/// Commands Rust may send. Mirrors the Swift `HostCommand` exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum HostCommand {
    /// Check the helper is alive and report its version.
    Version,
    /// Build + validate a VM configuration without starting it.
    Validate {
        params: CreateParams,
    },
    /// Register a computer (paths must already exist; Rust owns layout).
    Create {
        params: CreateParams,
    },
    Start {
        computer_id: String,
    },
    Pause {
        computer_id: String,
    },
    Resume {
        computer_id: String,
    },
    /// Graceful stop if the guest cooperates, forced otherwise.
    Stop {
        computer_id: String,
    },
    State {
        computer_id: String,
    },
    /// Stop if running and forget the VM object (Rust deletes files).
    Destroy {
        computer_id: String,
    },
    /// Queue one JSONL frame for the guest over vsock. Payload only;
    /// never a generic command channel (see docs/GUEST_PROTOCOL.md).
    GuestSend {
        computer_id: String,
        payload: String,
    },
    /// Whether a guest vsock connection is currently established.
    GuestStatus {
        computer_id: String,
    },
    /// Drop the guest connection from the host side.
    GuestDisconnect {
        computer_id: String,
    },
    /// Connect the helper to the guest's egress listener (vsock port
    /// 4051) and bridge the raw bytes to `endpoint`, which the app has
    /// already created (macOS: a Unix socket inside the Pegoles data
    /// folder; Windows: `\\.\pipe\pegoles-egress-<uuid>`). Transport
    /// only, at most one stream per computer. See docs/EGRESS.md.
    EgressOpen {
        computer_id: String,
        endpoint: String,
    },
    /// Close the egress stream (both sides); idempotent.
    EgressClose {
        computer_id: String,
    },
}

/// Paths and resources for one computer. All paths are inside that
/// computer's Application Support directory; Rust creates the layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    pub computer_id: String,
    pub disk_path: String,
    pub efi_vars_path: String,
    pub machine_id_path: String,
    pub serial_log_path: String,
    pub vcpus: u8,
    pub memory_mb: u32,
    /// Build-time provisioning only (cloud-init seed ISO). None in the
    /// normal lifecycle; the guest boots from disk_path alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_iso_path: Option<String>,
    /// Requested guest framebuffer. Informational today: the macOS helper
    /// always attaches one virtio-gpu scanout (1440x900) and NO host
    /// keyboard/pointing devices; agent input travels the guest channel
    /// (uinput inside the guest). There is no command to inject input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayParams>,
}

/// Scanout size for the guest framebuffer (validated on both sides).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayParams {
    pub width_px: u32,
    pub height_px: u32,
}

/// One request line: `{ "id": n, "command": ..., ... }`.
/// `HostCommand` is externally tagged, so we wrap id + command flattened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRequest {
    pub id: u64,
    #[serde(flatten)]
    pub command: HostCommand,
}

/// Machine-readable error codes (must match Swift `HostErrorCode`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostErrorCode {
    UnknownCommand,
    UnknownComputer,
    AlreadyExists,
    InvalidParams,
    ValidationFailed,
    StartFailed,
    StopFailed,
    PauseFailed,
    ResumeFailed,
    NotEntitled,
    /// No guest vsock connection established right now.
    GuestUnavailable,
    /// The egress stream could not be opened (guest not listening, bad
    /// endpoint, already open).
    EgressFailed,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostError {
    pub code: HostErrorCode,
    pub message: String,
}

/// VM states on the wire. Subset of `VZVirtualMachine.State` mapped by Swift:
/// starting/pausing/resuming -> "starting", stopping -> "stopping".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmState {
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
    Error,
}

impl VmState {
    pub fn as_protocol(&self) -> pegoles_protocol::ComputerState {
        use pegoles_protocol::ComputerState as C;
        match self {
            VmState::Stopped => C::Stopped,
            VmState::Starting => C::Starting,
            VmState::Running => C::Running,
            VmState::Paused => C::Paused,
            VmState::Stopping => C::Stopping,
            VmState::Error => C::Error,
        }
    }
}

/// One response line: `{ "id": n, "ok": true, ... }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<VmState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<HostError>,
}

/// Async lines Swift may emit at any time (`"id"` absent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HostEvent {
    VmStateChanged {
        computer_id: String,
        state: VmState,
    },
    VmFailed {
        computer_id: String,
        message: String,
    },
    /// Guest opened the vsock control channel (readiness begins).
    GuestConnected {
        computer_id: String,
    },
    /// One complete guest frame (already length/UTF-8 checked by Swift).
    GuestFrame {
        computer_id: String,
        payload: String,
    },
    /// Guest channel closed. `reason`: eof | frame_too_large |
    /// invalid_utf8 | kicked | helper_gone.
    GuestDisconnected {
        computer_id: String,
        reason: String,
    },
    /// The egress stream ended by itself (guest or app side closed, VM
    /// stopped). Not sent in reply to `egress_close`.
    EgressClosed {
        computer_id: String,
        reason: String,
    },
}

/// Classify one stdout line from the helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostLine {
    Response(HostResponse),
    Event(HostEvent),
}

pub fn format_request(id: u64, command: &HostCommand) -> String {
    let req = HostRequest {
        id,
        command: command.clone(),
    };
    serde_json::to_string(&req).expect("request is serializable")
}

/// Parse one helper stdout line. Garbage is an error, never trusted data.
pub fn parse_line(line: &str) -> Result<HostLine, String> {
    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("event").is_some() {
        let ev: HostEvent = serde_json::from_value(v).map_err(|e| format!("bad event: {e}"))?;
        Ok(HostLine::Event(ev))
    } else if v.get("id").is_some() {
        let resp: HostResponse =
            serde_json::from_value(v).map_err(|e| format!("bad response: {e}"))?;
        Ok(HostLine::Response(resp))
    } else {
        Err("line is neither response nor event".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> CreateParams {
        CreateParams {
            computer_id: "c1".into(),
            disk_path: "/tmp/disk.img".into(),
            efi_vars_path: "/tmp/efi-vars.bin".into(),
            machine_id_path: "/tmp/machine-id".into(),
            serial_log_path: "/tmp/serial.log".into(),
            vcpus: 2,
            memory_mb: 1536,
            seed_iso_path: None,
            display: None,
        }
    }

    #[test]
    fn request_line_shape() {
        let line = format_request(
            7,
            &HostCommand::Start {
                computer_id: "c1".into(),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["command"], "start");
        assert_eq!(v["computer_id"], "c1");
    }

    #[test]
    fn no_shell_command_exists() {
        // Guard: the wire protocol must never grow host execution.
        let json = serde_json::to_string(&HostCommand::Start {
            computer_id: "x".into(),
        })
        .unwrap()
        .to_lowercase();
        for forbidden in ["shell", "exec", "host", "proxy", "network", "file"] {
            assert!(!json.contains(forbidden), "contains {forbidden}");
        }
        let all = [
            "version",
            "validate",
            "create",
            "start",
            "pause",
            "resume",
            "stop",
            "state",
            "destroy",
            "guest_send",
            "guest_status",
            "guest_disconnect",
            "egress_open",
            "egress_close",
        ];
        assert_eq!(all.len(), 14);
    }

    /// Wire name of every command. The match is exhaustive on purpose:
    /// adding a variant fails to compile until it is listed here, which
    /// forces it through the guards below.
    fn command_wire_name(c: &HostCommand) -> &'static str {
        match c {
            HostCommand::Version => "version",
            HostCommand::Validate { .. } => "validate",
            HostCommand::Create { .. } => "create",
            HostCommand::Start { .. } => "start",
            HostCommand::Pause { .. } => "pause",
            HostCommand::Resume { .. } => "resume",
            HostCommand::Stop { .. } => "stop",
            HostCommand::State { .. } => "state",
            HostCommand::Destroy { .. } => "destroy",
            HostCommand::GuestSend { .. } => "guest_send",
            HostCommand::GuestStatus { .. } => "guest_status",
            HostCommand::GuestDisconnect { .. } => "guest_disconnect",
            HostCommand::EgressOpen { .. } => "egress_open",
            HostCommand::EgressClose { .. } => "egress_close",
        }
    }

    fn one_of_each_command() -> Vec<HostCommand> {
        let cid = || "c".to_string();
        vec![
            HostCommand::Version,
            HostCommand::Validate { params: params() },
            HostCommand::Create { params: params() },
            HostCommand::Start { computer_id: cid() },
            HostCommand::Pause { computer_id: cid() },
            HostCommand::Resume { computer_id: cid() },
            HostCommand::Stop { computer_id: cid() },
            HostCommand::State { computer_id: cid() },
            HostCommand::Destroy { computer_id: cid() },
            HostCommand::GuestSend {
                computer_id: cid(),
                payload: "{}".into(),
            },
            HostCommand::GuestStatus { computer_id: cid() },
            HostCommand::GuestDisconnect { computer_id: cid() },
            HostCommand::EgressOpen {
                computer_id: cid(),
                endpoint: "/tmp/e/s".into(),
            },
            HostCommand::EgressClose { computer_id: cid() },
        ]
    }

    /// SECURITY guard (Phase 4): the VM host protocol must never grow an
    /// input-injection command. Human input reaches the guest only through
    /// the native view while the user explicitly took control; nothing on
    /// this wire can synthesize pointer/keyboard events.
    #[test]
    fn no_input_injection_command_exists() {
        let commands = one_of_each_command();
        assert_eq!(commands.len(), 14);
        for command in &commands {
            let line = format_request(1, command);
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            let tag = v["command"].as_str().unwrap().to_string();
            assert_eq!(tag, command_wire_name(command));
            for forbidden in [
                "input", "inject", "pointer", "mouse", "keyboard", "key", "type", "click",
                "scroll", "touch", "cursor", "synth",
            ] {
                assert!(!tag.contains(forbidden), "command {tag} looks like input");
            }
        }
        // Source scan of the enum block too (doc comments may explain the
        // rule, variant identifiers may not carry it).
        // Normalize CRLF checkouts (Windows CI) before splitting on lines.
        let src = include_str!("vmhost_proto.rs").replace("\r\n", "\n");
        let block = src
            .split("pub enum HostCommand {")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("HostCommand enum block");
        for line in block.lines() {
            let code = line.trim();
            if code.starts_with("///") || code.starts_with("//") {
                continue;
            }
            let lower = code.to_lowercase();
            for forbidden in [
                "inject",
                "pointer",
                "mouse",
                "keyboard",
                "click",
                "type_text",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "HostCommand variant looks like input: {code}"
                );
            }
        }
    }

    #[test]
    fn guest_commands_have_no_generic_channel() {
        // The only guest-bound command carries an opaque payload frame;
        // there is no exec/shell/eval/native capability on the wire.
        let line = format_request(
            9,
            &HostCommand::GuestSend {
                computer_id: "c".into(),
                payload: "{}".into(),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["command"], "guest_send");
        for forbidden in ["exec", "shell", "eval", "native", "raw_command"] {
            assert!(!line.to_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn parses_ok_response() {
        let line = r#"{"id":3,"ok":true,"state":"running"}"#;
        match parse_line(line).unwrap() {
            HostLine::Response(r) => {
                assert_eq!(r.id, 3);
                assert!(r.ok);
                assert_eq!(r.state, Some(VmState::Running));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_error_response() {
        let line = r#"{"id":4,"ok":false,"error":{"code":"unknown_computer","message":"nope"}}"#;
        match parse_line(line).unwrap() {
            HostLine::Response(r) => {
                assert!(!r.ok);
                let e = r.error.unwrap();
                assert_eq!(e.code, HostErrorCode::UnknownComputer);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_async_events() {
        match parse_line(r#"{"event":"vm_state_changed","computer_id":"c","state":"paused"}"#)
            .unwrap()
        {
            HostLine::Event(HostEvent::VmStateChanged { state, .. }) => {
                assert_eq!(state, VmState::Paused)
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse_line(r#"{"event":"vm_failed","computer_id":"c","message":"boom"}"#).unwrap() {
            HostLine::Event(HostEvent::VmFailed { .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
        match parse_line(r#"{"event":"guest_connected","computer_id":"c"}"#).unwrap() {
            HostLine::Event(HostEvent::GuestConnected { .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
        match parse_line(
            r#"{"event":"guest_frame","computer_id":"c","payload":"{\"type\":\"ready\"}"}"#,
        )
        .unwrap()
        {
            HostLine::Event(HostEvent::GuestFrame { payload, .. }) => {
                assert!(payload.contains("ready"))
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse_line(r#"{"event":"guest_disconnected","computer_id":"c","reason":"eof"}"#)
            .unwrap()
        {
            HostLine::Event(HostEvent::GuestDisconnected { reason, .. }) => {
                assert_eq!(reason, "eof")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn egress_request_shapes() {
        let line = format_request(
            3,
            &HostCommand::EgressOpen {
                computer_id: "c1".into(),
                endpoint: r"\\.\pipe\pegoles-egress-x".into(),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["command"], "egress_open");
        assert_eq!(v["computer_id"], "c1");
        assert_eq!(v["endpoint"], r"\\.\pipe\pegoles-egress-x");
        let line = format_request(
            4,
            &HostCommand::EgressClose {
                computer_id: "c1".into(),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["command"], "egress_close");
        assert_eq!(v.as_object().unwrap().len(), 3);
        // A request without an endpoint is not an egress_open.
        assert!(serde_json::from_str::<HostRequest>(
            r#"{"id":1,"command":"egress_open","computer_id":"c"}"#
        )
        .is_err());
    }

    #[test]
    fn egress_replies_and_events_parse() {
        match parse_line(
            r#"{"id":5,"ok":false,"error":{"code":"egress_failed","message":"guest is not listening"}}"#,
        )
        .unwrap()
        {
            HostLine::Response(r) => {
                assert_eq!(r.error.unwrap().code, HostErrorCode::EgressFailed)
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse_line(r#"{"event":"egress_closed","computer_id":"c","reason":"guest_closed"}"#)
            .unwrap()
        {
            HostLine::Event(HostEvent::EgressClosed {
                computer_id,
                reason,
            }) => {
                assert_eq!(
                    (computer_id.as_str(), reason.as_str()),
                    ("c", "guest_closed")
                )
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_guest_status_response() {
        match parse_line(r#"{"id":5,"ok":true,"connected":true}"#).unwrap() {
            HostLine::Response(r) => assert_eq!(r.connected, Some(true)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage_and_unknown_shapes() {
        assert!(parse_line("not json").is_err());
        assert!(parse_line(r#"{"hello":1}"#).is_err());
        assert!(parse_line(r#"{"event":"execute_arbitrary_host_shell"}"#).is_err());
    }

    #[test]
    fn vm_state_maps_to_protocol() {
        use pegoles_protocol::ComputerState as C;
        assert_eq!(VmState::Running.as_protocol(), C::Running);
        assert_eq!(VmState::Starting.as_protocol(), C::Starting);
        assert_eq!(VmState::Stopping.as_protocol(), C::Stopping);
        assert_eq!(VmState::Error.as_protocol(), C::Error);
    }

    #[test]
    fn create_params_round_trip() {
        let p = params();
        let s = serde_json::to_string(&p).unwrap();
        let back: CreateParams = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}

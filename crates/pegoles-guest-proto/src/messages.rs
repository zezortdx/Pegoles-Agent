//! Guest protocol constants + v0.1 message types (JSON Lines).
//!
//! Versioning: `GUEST_PROTOCOL_VERSION` is negotiated on every handshake.
//! Host speaks 1; a guest offering anything else is `Incompatible`, never
//! silently accepted.

use serde::{Deserialize, Serialize};

/// Canonical vsock port. Mirrored in the macOS helper and docs;
/// this constant is the single source of truth — no magic numbers.
pub const PEGOLES_VSOCK_PORT: u32 = 4050;

/// Wire protocol version spoken by this host release.
pub const GUEST_PROTOCOL_VERSION: u32 = 1;

/// Guest runtime release this host was built alongside (informational).
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Guest -> Host messages. The guest MUST send `GuestHello` first on every
/// new connection; anything else first is a protocol violation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuestMessage {
    GuestHello {
        protocol_version: u32,
        runtime_version: String,
        os: String,
        os_version: String,
        arch: String,
        /// Phase 5 capability advertisement, e.g. `["input", "frame"]`.
        /// Optional so v0.1 hellos still parse; unknown entries ignored.
        #[serde(default)]
        capabilities: Vec<String>,
    },
    /// Handshake complete from the guest side (after receiving HostHello).
    Ready,
    /// Echo of `HostMessage::Ping.nonce`.
    Pong { nonce: u64 },
    /// Answer to `HostMessage::GetSystemInfo`.
    SystemInfo(SystemInfo),
    /// Guest-side fault report (diagnostics only, never trusted blindly).
    Error { code: String, message: String },
    /// Graphical session (compositor) status. Pushed by the guest whenever
    /// it changes and sent in answer to `GetGraphicalSession`. A STATUS
    /// report, not a capability: it carries no input, no pixels, no
    /// commands. Older guests never send it (host treats the session as
    /// unavailable); older hosts ignore it (unknown type).
    GraphicalSession(GraphicalSessionReport),
    /// Phase 5: acknowledgement of `HostMessage::Input`. Stateless
    /// routing: the host matches `request_id` to its pending action.
    InputAck {
        request_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Phase 5: one base64 chunk of a captured frame's raw RGBA bytes
    /// (row-major, top-left). Preceded by `FrameBegin`, followed by the
    /// host reassembling `total` chunks in `seq` order.
    FrameBegin {
        request_id: String,
        width_px: u32,
        height_px: u32,
        total_chunks: u32,
    },
    FrameChunk {
        request_id: String,
        seq: u32,
        /// base64 of ≤32 KiB raw RGBA.
        bytes: String,
    },
}

/// Host -> Guest messages. v0.1 was capability-free; Phase 5 adds the
/// GUEST-ONLY input/observation verbs below. They address the isolated
/// guest's compositor/input stack and can never name host resources.
/// Protocol version stays 1: unknown types are ignored by older guests
/// (forward compatibility), so capability is negotiated by trying.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    HostHello {
        protocol_version: u32,
    },
    Ping {
        nonce: u64,
    },
    GetSystemInfo,
    Error {
        code: String,
        message: String,
    },
    /// Ask for the current `GraphicalSession` report (status only).
    GetGraphicalSession,
    /// Execute one guest input primitive (guest pixels, canonical keys).
    Input {
        request_id: String,
        op: GuestInputOp,
        /// Guest framebuffer size the host converted from. The guest
        /// validates touchscreen scaling against it (and its own cached
        /// output size); mismatches are acked as errors, never guessed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<GuestDisplaySize>,
    },
    /// Capture the guest framebuffer (VM display only) as raw RGBA
    /// chunks. On demand; the guest answers FrameBegin + FrameChunks.
    GetFrame {
        request_id: String,
    },
}

/// One input primitive in GUEST pixels (converted from agent space by
/// the host executor with the current display size).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GuestInputOp {
    Move {
        x: u32,
        y: u32,
    },
    Press {
        x: u32,
        y: u32,
        button: GuestButton,
    },
    Release {
        x: u32,
        y: u32,
        button: GuestButton,
    },
    Click {
        x: u32,
        y: u32,
        button: GuestButton,
    },
    DoubleClick {
        x: u32,
        y: u32,
        button: GuestButton,
    },
    Drag {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        button: GuestButton,
        duration_ms: u32,
    },
    Scroll {
        x: u32,
        y: u32,
        dx: i32,
        dy: i32,
    },
    Key {
        key: String,
        down: bool,
    },
    Chord {
        keys: Vec<String>,
    },
    Type {
        text: String,
    },
}

/// Guest framebuffer size accompanying an input request (scaling facts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestDisplaySize {
    pub width_px: u32,
    pub height_px: u32,
}

/// Guest pointer buttons. Primary is the guest's main button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestButton {
    Primary,
    Secondary,
    Middle,
}

/// Capability names a Phase 5 guest may advertise in `GuestHello`.
pub const GUEST_CAP_INPUT: &str = "input";
pub const GUEST_CAP_FRAME: &str = "frame";

/// Cap for capability entries (count + bytes each).
pub const MAX_CAPABILITIES: usize = 8;
pub const MAX_CAPABILITY_BYTES: usize = 32;

/// Whether an advertised capability list is well-formed (untrusted).
pub fn capabilities_bounded(caps: &[String]) -> bool {
    caps.len() <= MAX_CAPABILITIES && caps.iter().all(|c| c.len() <= MAX_CAPABILITY_BYTES)
}

/// Bounds for untrusted Phase 5 fields.
pub const MAX_REQUEST_ID_BYTES: usize = 64;
pub const MAX_TYPE_TEXT_CHARS: usize = 4096;
pub const MAX_CHORD_KEYS: usize = 4;
pub const MAX_FRAME_CHUNK_B64: usize = 48 * 1024;
pub const MAX_FRAME_CHUNKS: u32 = 1024;
pub const MAX_KEY_NAME_BYTES: usize = 32;

impl GuestInputOp {
    /// Whether every field respects the untrusted-input bounds.
    pub fn is_bounded(&self) -> bool {
        match self {
            GuestInputOp::Move { .. }
            | GuestInputOp::Press { .. }
            | GuestInputOp::Release { .. }
            | GuestInputOp::Click { .. }
            | GuestInputOp::DoubleClick { .. } => true,
            GuestInputOp::Drag { duration_ms, .. } => {
                *duration_ms <= pegoles_protocol_limits::MAX_DRAG_MS
            }
            GuestInputOp::Scroll { dx, dy, .. } => {
                (*dx as f64).abs() <= pegoles_protocol_limits::MAX_SCROLL_UNITS
                    && (*dy as f64).abs() <= pegoles_protocol_limits::MAX_SCROLL_UNITS
            }
            GuestInputOp::Key { key, .. } => key.len() <= MAX_KEY_NAME_BYTES,
            GuestInputOp::Chord { keys } => {
                keys.len() <= MAX_CHORD_KEYS && keys.iter().all(|k| k.len() <= MAX_KEY_NAME_BYTES)
            }
            GuestInputOp::Type { text } => text.chars().count() <= MAX_TYPE_TEXT_CHARS,
        }
    }
}

/// Bounds mirror of `pegoles-protocol::limits` (kept dependency-free so
/// the guest binary stays lean). Cross-crate equality is asserted by
/// `pegoles-computer/src/input.rs::limits_mirror_protocol`.
mod pegoles_protocol_limits {
    pub const MAX_DRAG_MS: u32 = 10_000;
    pub const MAX_SCROLL_UNITS: f64 = 100.0;
}

/// Compositor status as observed INSIDE the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicalSessionStatus {
    /// No graphical session configured (headless image / no display).
    Unavailable,
    /// Compositor launching, not accepting clients yet.
    Starting,
    /// Compositor completed a real client round-trip.
    Ready,
    /// Compositor failed or exited.
    Failed,
}

/// Bounded report (`MAX_SESSION_FIELD_BYTES` per string field).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicalSessionReport {
    pub status: GraphicalSessionStatus,
    /// Compositor name, e.g. "weston". Informational.
    pub compositor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    /// Short diagnostic (failure reason). Never command output or paths
    /// from user data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Cap for each string field of `GraphicalSessionReport`.
pub const MAX_SESSION_FIELD_BYTES: usize = 256;

impl GraphicalSessionReport {
    /// Whether every string field respects the bound (untrusted input).
    pub fn is_bounded(&self) -> bool {
        self.compositor.len() <= MAX_SESSION_FIELD_BYTES
            && self
                .detail
                .as_ref()
                .map(|d| d.len() <= MAX_SESSION_FIELD_BYTES)
                .unwrap_or(true)
    }
}

/// Aggregate CPU time counters (clock ticks since boot, /proc/stat
/// semantics). Two samples give guest-internal CPU utilization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuJiffies {
    pub busy: u64,
    pub total: u64,
}

/// Resident memory of one ALLOWLISTED guest process (e.g. "weston").
/// Name only — never command lines, arguments, or environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRss {
    pub name: String,
    pub rss_kb: u64,
}

/// Minimal system facts. Explicitly excludes: home contents, environment
/// variables, credentials, tokens, process command lines, personal data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub os_version: String,
    pub kernel: String,
    pub arch: String,
    pub hostname: String,
    pub runtime_version: String,
    pub protocol_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_total_mb: Option<u64>,
    // --- Phase 4 measurement facts (allowlist; all optional so older
    // runtimes stay compatible). Used by bench/PERFORMANCE.md, never by
    // agent logic. ---
    /// MemAvailable from /proc/meminfo, in MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_available_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_jiffies: Option<CpuJiffies>,
    /// RSS of allowlisted processes only (compositor, terminal, browser,
    /// runtime). Bounded list; unknown names are never reported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_rss: Vec<ProcessRss>,
}

/// Parse one complete frame line into a guest message.
/// Unknown `type` values are a distinct error so the caller can ignore
/// (forward compatibility) rather than drop the connection.
pub fn parse_guest_message(line: &str) -> Result<GuestMessage, MessageError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| MessageError::InvalidJson)?;
    match value.get("type").and_then(|t| t.as_str()) {
        Some("guest_hello")
        | Some("ready")
        | Some("pong")
        | Some("system_info")
        | Some("error")
        | Some("graphical_session")
        | Some("input_ack")
        | Some("frame_begin")
        | Some("frame_chunk") => {
            serde_json::from_value(value).map_err(|_| MessageError::InvalidShape)
        }
        _ => Err(MessageError::UnknownType),
    }
}

pub fn parse_host_message(line: &str) -> Result<HostMessage, MessageError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| MessageError::InvalidJson)?;
    match value.get("type").and_then(|t| t.as_str()) {
        Some("host_hello")
        | Some("ping")
        | Some("get_system_info")
        | Some("error")
        | Some("get_graphical_session")
        | Some("input")
        | Some("get_frame") => {
            serde_json::from_value(value).map_err(|_| MessageError::InvalidShape)
        }
        _ => Err(MessageError::UnknownType),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageError {
    InvalidJson,
    InvalidShape,
    UnknownType,
}

/// Serialize a host message as one JSONL frame (no trailing newline;
/// the transport adds it).
pub fn encode_host(msg: &HostMessage) -> String {
    serde_json::to_string(msg).expect("host message serializes")
}

pub fn encode_guest(msg: &GuestMessage) -> String {
    serde_json::to_string(msg).expect("guest message serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_hello_shape() {
        let m = GuestMessage::GuestHello {
            protocol_version: 1,
            runtime_version: "0.1.0".into(),
            os: "debian".into(),
            os_version: "13".into(),
            arch: "aarch64".into(),
            capabilities: vec!["input".into(), "frame".into()],
        };
        let line = encode_guest(&m);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "guest_hello");
        assert_eq!(parse_guest_message(&line).unwrap(), m);
        // v0.1 hellos without capabilities still parse.
        let legacy = r#"{"type":"guest_hello","protocol_version":1,"runtime_version":"0.1.0","os":"debian","os_version":"13","arch":"aarch64"}"#;
        match parse_guest_message(legacy).unwrap() {
            GuestMessage::GuestHello { capabilities, .. } => assert!(capabilities.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(capabilities_bounded(&["input".to_string()]));
        assert!(!capabilities_bounded(&["x".repeat(33)]));
    }

    #[test]
    fn host_messages_round_trip() {
        for m in [
            HostMessage::HostHello {
                protocol_version: 1,
            },
            HostMessage::Ping { nonce: 42 },
            HostMessage::GetSystemInfo,
            HostMessage::Error {
                code: "x".into(),
                message: "y".into(),
            },
        ] {
            let line = encode_host(&m);
            assert!(!line.contains('\n'));
            assert_eq!(parse_host_message(&line).unwrap(), m);
        }
    }

    #[test]
    fn unknown_type_is_distinct_from_garbage() {
        assert_eq!(
            parse_guest_message(r#"{"type":"do_evil","x":1}"#),
            Err(MessageError::UnknownType)
        );
        assert_eq!(
            parse_guest_message("not json"),
            Err(MessageError::InvalidJson)
        );
        assert_eq!(
            parse_guest_message(r#"{"type":"ready","extra":[1,2"#),
            Err(MessageError::InvalidJson)
        );
        assert_eq!(
            parse_guest_message(r#"{"type":"pong"}"#),
            Err(MessageError::InvalidShape)
        );
    }

    #[test]
    fn no_capability_creep_in_v01() {
        // Guard, Phase 5 update: the ONLY execution-adjacent verbs on the
        // wire are the documented guest-ONLY input/observation set. Host
        // execution, shell, filesystem, and process verbs stay forbidden.
        for sample in [
            encode_host(&HostMessage::HostHello {
                protocol_version: 1,
            }),
            encode_host(&HostMessage::Ping { nonce: 1 }),
            encode_host(&HostMessage::GetSystemInfo),
            encode_host(&HostMessage::GetGraphicalSession),
            encode_host(&HostMessage::Input {
                request_id: "r".into(),
                op: GuestInputOp::Click {
                    x: 1,
                    y: 2,
                    button: GuestButton::Primary,
                },
                display: None,
            }),
            encode_host(&HostMessage::GetFrame {
                request_id: "r".into(),
            }),
        ] {
            let lower = sample.to_lowercase();
            // `host_hello` is the pre-existing handshake name; what stays
            // forbidden is host EXECUTION (shell/process/command/file).
            for forbidden in ["shell", "exec", "file", "process", "install", "command"] {
                assert!(!lower.contains(forbidden), "leaked {forbidden} in {sample}");
            }
        }
        // And the parser accepts exactly the documented set.
        assert_eq!(
            parse_host_message(&encode_host(&HostMessage::GetFrame {
                request_id: "r".into()
            }))
            .unwrap(),
            HostMessage::GetFrame {
                request_id: "r".into()
            }
        );
    }

    #[test]
    fn input_ops_round_trip_and_bound() {
        let ops = [
            GuestInputOp::Move { x: 720, y: 450 },
            GuestInputOp::Press {
                x: 1,
                y: 2,
                button: GuestButton::Secondary,
            },
            GuestInputOp::Release {
                x: 1,
                y: 2,
                button: GuestButton::Middle,
            },
            GuestInputOp::Click {
                x: 1,
                y: 2,
                button: GuestButton::Primary,
            },
            GuestInputOp::DoubleClick {
                x: 1,
                y: 2,
                button: GuestButton::Primary,
            },
            GuestInputOp::Drag {
                from_x: 0,
                from_y: 0,
                to_x: 100,
                to_y: 100,
                button: GuestButton::Primary,
                duration_ms: 400,
            },
            GuestInputOp::Scroll {
                x: 1,
                y: 2,
                dx: 0,
                dy: -3,
            },
            GuestInputOp::Key {
                key: "Enter".into(),
                down: true,
            },
            GuestInputOp::Chord {
                keys: vec!["Control".into(), "c".into()],
            },
            GuestInputOp::Type {
                text: "echo hello".into(),
            },
        ];
        for op in ops {
            assert!(op.is_bounded());
            let m = HostMessage::Input {
                request_id: "req-1".into(),
                op,
                display: Some(GuestDisplaySize {
                    width_px: 1440,
                    height_px: 900,
                }),
            };
            let line = encode_host(&m);
            assert!(!line.contains('\n'));
            assert_eq!(parse_host_message(&line).unwrap(), m);
        }
        assert!(!GuestInputOp::Type {
            text: "x".repeat(MAX_TYPE_TEXT_CHARS + 1)
        }
        .is_bounded());
        assert!(!GuestInputOp::Drag {
            from_x: 0,
            from_y: 0,
            to_x: 1,
            to_y: 1,
            button: GuestButton::Primary,
            duration_ms: 60_000,
        }
        .is_bounded());
        let ack = GuestMessage::InputAck {
            request_id: "req-1".into(),
            ok: true,
            error: None,
        };
        assert_eq!(parse_guest_message(&encode_guest(&ack)).unwrap(), ack);
        let chunk = GuestMessage::FrameChunk {
            request_id: "f".into(),
            seq: 0,
            bytes: "aGVsbG8=".into(),
        };
        assert_eq!(parse_guest_message(&encode_guest(&chunk)).unwrap(), chunk);
    }

    #[test]
    fn graphical_session_round_trips_and_is_bounded() {
        let m = GuestMessage::GraphicalSession(GraphicalSessionReport {
            status: GraphicalSessionStatus::Ready,
            compositor: "weston".into(),
            width_px: Some(1440),
            height_px: Some(900),
            detail: None,
        });
        let line = encode_guest(&m);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "graphical_session");
        assert_eq!(v["status"], "ready");
        assert_eq!(parse_guest_message(&line).unwrap(), m);
        let long = GraphicalSessionReport {
            status: GraphicalSessionStatus::Failed,
            compositor: "x".repeat(MAX_SESSION_FIELD_BYTES + 1),
            width_px: None,
            height_px: None,
            detail: None,
        };
        assert!(!long.is_bounded());
        assert_eq!(
            parse_host_message(&encode_host(&HostMessage::GetGraphicalSession)).unwrap(),
            HostMessage::GetGraphicalSession
        );
    }

    #[test]
    fn system_info_minimal_shape() {
        let info = SystemInfo {
            os: "debian".into(),
            os_version: "13".into(),
            kernel: "6.12.0".into(),
            arch: "aarch64".into(),
            hostname: "pegoles".into(),
            runtime_version: "0.1.0".into(),
            protocol_version: 1,
            uptime_s: None,
            cpu_count: None,
            mem_total_mb: None,
            mem_available_mb: None,
            cpu_jiffies: None,
            process_rss: Vec::new(),
        };
        let line = serde_json::to_string(&info).unwrap();
        assert!(!line.contains("home"));
        assert!(!line.contains("token"));
    }
}

//! Pegoles Core <-> Guest Runtime protocol v0.1.
//!
//! This is a DIFFERENT protocol from the host<->VM-helper protocol
//! (`vmhost_proto.rs`):
//! - host<->helper: VM lifecycle control plane (child process JSONL).
//! - Core<->Guest: agent control plane over AF_VSOCK (virtio socket).
//!
//! Transport: JSON Lines. Sockets are streams, so framing is explicit
//! (`framing`): one UTF-8 line per message, 64 KiB cap, partial reads and
//! batched reads supported, oversized/invalid frames rejected.
//!
//! SECURITY: every inbound field is untrusted guest input. Lengths are
//! bounded, strings are length-checked on use, unknown message types are
//! ignored (never executed), and there is no shell/file/process capability
//! in this version by design.

pub mod framing;
pub mod messages;

pub use framing::{FrameError, Framer, MAX_FRAME_BYTES};
pub use messages::{
    capabilities_bounded, diagnostics_bounded, encode_guest, encode_host, parse_guest_message,
    parse_host_message, CapabilityDiagnostic, CpuJiffies, GraphicalSessionReport,
    GraphicalSessionStatus, GuestButton, GuestDisplaySize, GuestInputOp, GuestMessage, HostMessage,
    MessageError, ProcessRss, SystemInfo, CLICK_HOLD_MS, DOUBLE_CLICK_GAP_MS, GUEST_CAP_FRAME,
    GUEST_CAP_INPUT, GUEST_PROTOCOL_VERSION, KEY_STROKE_PACING_MS, MAX_CAPABILITIES,
    MAX_CAPABILITY_BYTES, MAX_CHORD_KEYS, MAX_DIAGNOSTICS, MAX_DIAGNOSTIC_BYTES, MAX_FRAME_CHUNKS,
    MAX_FRAME_CHUNK_B64, MAX_KEY_NAME_BYTES, MAX_REQUEST_ID_BYTES, MAX_RSS_PROCESSES,
    MAX_SESSION_FIELD_BYTES, MAX_STROKES_PER_CHAR, MAX_TYPE_TEXT_CHARS, PEGOLES_VSOCK_PORT,
    RUNTIME_VERSION,
};

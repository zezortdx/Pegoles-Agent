//! Pegoles egress mux protocol (docs/EGRESS.md, "Mux protocol").
//!
//! One byte stream (vsock / HvSocket, bridged by the VM helper) carries many
//! logical streams. Frame: `stream_id: u32 BE | kind: u8 | len: u32 BE |
//! payload[len]`, `len <= MAX_PAYLOAD`.
//!
//! SECURITY INVARIANTS:
//! - The peer is hostile: every length is checked before any allocation, the
//!   decoder never holds more than one frame, nothing panics on input.
//! - Any violation is an error the caller answers by dropping the whole
//!   connection. The decoder is poisoned after its first error.
//! - No dependencies: the same code is compiled for the Linux guest.

mod conn;
mod decoder;
mod frame;

pub use conn::{Conn, Event, Role};
pub use decoder::Decoder;
pub use frame::{Frame, Kind, ProtoError};

/// Protocol version carried in HELLO / HELLO_ACK.
pub const VERSION: u16 = 1;
/// vsock / HvSocket port the guest listens on for the egress channel.
pub const VSOCK_PORT: u32 = 4051;
/// TCP port of the guest-side forwarder (`127.0.0.1`), Chromium's proxy.
pub const GUEST_PROXY_PORT: u16 = 3128;
/// Bytes in a frame header.
pub const HEADER_LEN: usize = 9;
/// Largest frame payload.
pub const MAX_PAYLOAD: usize = 16384;
/// Largest CA certificate (DER) in HELLO.
pub const MAX_CA_DER: usize = 4096;
/// Initial per-stream window, in each direction.
pub const INITIAL_WINDOW: u32 = 262_144;
/// Most streams open at once.
pub const MAX_STREAMS: usize = 64;
/// Stream id of connection-level frames (HELLO, HELLO_ACK).
pub const CONTROL_STREAM: u32 = 0;

#[cfg(test)]
mod tests;

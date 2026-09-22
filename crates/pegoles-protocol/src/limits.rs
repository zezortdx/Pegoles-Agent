//! Safety caps for structured computer input (Phase 5).
//!
//! Single source of truth shared by Policy (static rejection), the Core
//! executor (runtime rate limiting), and the deterministic runner.
//! Reasonable limits, not user-hostile: groundwork for future LLM safety.

/// Maximum interpolated drag duration.
pub const MAX_DRAG_MS: u32 = 10_000;
/// Maximum scroll delta per action, in logical units.
pub const MAX_SCROLL_UNITS: f64 = 100.0;
/// Maximum characters per TypeText action.
pub const MAX_TYPE_CHARS: usize = 4096;
/// Maximum Wait duration per action.
pub const MAX_WAIT_MS: u32 = 30_000;
/// Maximum action dispatches per second per computer (burst brake).
pub const MAX_ACTIONS_PER_SEC: u32 = 20;
/// Maximum pointer interpolation steps per drag (bounds IPC chatter).
pub const MAX_DRAG_STEPS: usize = 64;
/// Frame chunk size for vsock transport (raw bytes; base64 + JSON must
/// stay under the 64 KiB guest frame cap).
pub const FRAME_CHUNK_BYTES: usize = 32 * 1024;
/// Guest input round-trip timeout per primitive.
pub const INPUT_ROUNDTRIP_MS: u64 = 5_000;
/// Frame capture timeout (chunked transfers of multi-MB frames).
pub const FRAME_CAPTURE_TIMEOUT_MS: u64 = 30_000;
/// Maximum decoded frame payload the host will reassemble (64 MiB).
pub const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;

// Compile-time sanity on the caps above (fail the build, not a test).
const _: () = assert!(MAX_DRAG_MS >= 1_000);
const _: () = assert!(MAX_TYPE_CHARS >= 1_024);
const _: () = assert!(MAX_WAIT_MS >= 1_000);
const _: () = assert!(MAX_ACTIONS_PER_SEC >= 1);
const _: () = assert!(FRAME_CHUNK_BYTES < 64 * 1024);

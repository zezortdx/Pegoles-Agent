//! macOS adapter crate (Phase 4). CONTRACT SKELETON — implemented by the
//! native-embed stream:
//!
//! - `EmbeddedHostTransport`: `pegoles_computer::native_backend::HostTransport`
//!   over the in-process Swift VM host (same JSONL protocol, same closed
//!   command set as the `pegoles-vm-host` child process).
//! - `MacVirtualMachineDisplay`: `pegoles_computer::ComputerDisplayBackend`
//!   that places the native framebuffer view inside the Pegoles window.
//!
//! Every item is `cfg(target_os = "macos")`; other targets build an empty
//! crate so the shared workspace stays portable.

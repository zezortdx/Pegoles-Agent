//! Small audited helpers for the helper's Windows modules.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// A kernel handle closed exactly once.
pub struct Owned(pub HANDLE);

impl Owned {
    /// The handle (a method, so closures capture the owner, not the field).
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            // SAFETY: we own this handle and close it once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

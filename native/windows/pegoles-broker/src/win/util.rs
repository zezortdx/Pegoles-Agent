//! Small, audited helpers shared by the broker's Windows modules.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};

/// NUL-terminated UTF-16 for Win32 calls.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// Read a NUL-terminated UTF-16 string the OS returned (bounded).
///
/// # Safety
/// `ptr` must be null or point to a NUL-terminated UTF-16 string.
pub unsafe fn from_pwstr(ptr: PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: caller guarantees NUL termination; the bound stops a runaway read.
    while len < 32 * 1024 && unsafe { *ptr.0.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` elements were just read from the same allocation.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr.0, len) })
}

/// Free memory the OS allocated with LocalAlloc (SIDs as strings, SDs).
///
/// # Safety
/// `ptr` must come from a LocalAlloc-returning API and not be freed twice.
pub unsafe fn local_free(ptr: *mut core::ffi::c_void) {
    if !ptr.is_null() {
        // SAFETY: per the contract above.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(ptr)));
        }
    }
}

/// A kernel handle closed exactly once.
pub struct Owned(pub HANDLE);

impl Owned {
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

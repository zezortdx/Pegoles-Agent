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

/// The broker trusts the helper beside it, and the service runs this
/// binary as SYSTEM: both are sound only in a folder that only
/// administrators can write. Release builds refuse to run outside
/// Program Files (the per-machine installer's location).
pub fn require_admin_only_folder() -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let exe = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    // SAFETY: known folder lookup for this process.
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None) }
        .map_err(|e| format!("cannot find Program Files: {e}"))?;
    // SAFETY: NUL-terminated string from the shell, freed once below.
    let program_files = unsafe { from_pwstr(path) };
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _)) };
    if inside(&exe.to_string_lossy(), &program_files) {
        Ok(())
    } else {
        Err(format!(
            "Pegoles must be installed in {program_files} (found in {}); reinstall it with the installer",
            exe.display()
        ))
    }
}

/// `path` is strictly inside `folder` (case-insensitive, whole components).
fn inside(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('\\').to_lowercase();
    let path = path.to_lowercase();
    !folder.is_empty()
        && path.len() > folder.len() + 1
        && path.starts_with(&folder)
        && path.as_bytes()[folder.len()] == b'\\'
        && !path.contains("\\..\\")
}

#[cfg(test)]
mod tests {
    use super::inside;

    #[test]
    fn only_paths_inside_the_folder_count() {
        let pf = r"C:\Program Files";
        assert!(inside(
            r"C:\Program Files\Pegoles Agent\pegoles-broker.exe",
            pf
        ));
        assert!(inside(
            r"c:\program files\Pegoles Agent\pegoles-broker.exe",
            r"C:\Program Files\"
        ));
        assert!(!inside(
            r"C:\Program Files (x86)\Pegoles\pegoles-broker.exe",
            pf
        ));
        assert!(!inside(r"C:\Program FilesX\pegoles-broker.exe", pf));
        assert!(!inside(r"C:\Users\ana\Pegoles\pegoles-broker.exe", pf));
        assert!(!inside(
            r"C:\Program Files\..\Users\ana\pegoles-broker.exe",
            pf
        ));
        assert!(!inside(r"C:\Program Files", pf));
        assert!(!inside(r"C:\x.exe", ""));
    }
}

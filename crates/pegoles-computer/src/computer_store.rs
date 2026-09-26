//! Computers on disk: resumed across app launches, never leaked.
//!
//! Core keeps one computer in memory, so without this every launch
//! cloned a new disk and orphaned the previous one. Now a create first
//! looks for a computer built from the current image and adopts it
//! (same disk, EFI store and machine identity: the environment resumes);
//! computers from other images are unusable (older runtimes cannot
//! authenticate) and are removed. A computer another live process holds
//! (exclusive lock on `<dir>/lock`) is never touched.

use std::fs::{File, OpenOptions};
use std::path::Path;

use pegoles_protocol::ComputerId;

use crate::error::{ComputerError, Result};

/// Exclusive ownership of one computer directory for this process's
/// lifetime (released on drop / process exit).
#[derive(Debug)]
pub struct ComputerLock {
    _file: File,
}

fn try_lock(dir: &Path) -> Result<Option<ComputerLock>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("lock"))
        .map_err(|e| ComputerError::Backend(format!("cannot open computer lock: {e}")))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(ComputerLock { _file: file })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(ComputerError::Backend(format!("cannot lock computer: {e}")))
        }
    }
}

/// Lock a freshly created computer directory.
pub fn lock_new(dir: &Path) -> Result<ComputerLock> {
    try_lock(dir)?.ok_or_else(|| ComputerError::Backend("new computer is already locked".into()))
}

/// Find a resumable computer for `image_id` (adopting and locking it) and
/// remove unlocked computers built from any other image. Directories
/// that do not look like Pegoles computers are left alone.
pub fn claim_existing(
    computers_dir: &Path,
    image_id: &str,
    disk_file_name: &str,
) -> Option<(ComputerId, ComputerLock)> {
    let entries = std::fs::read_dir(computers_dir).ok()?;
    let mut adopted = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let Some(id) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<ComputerId>().ok())
        else {
            continue;
        };
        let metadata = dir.join("metadata.json");
        let disk = dir.join(disk_file_name);
        if !is_dir || !(metadata.exists() || disk.exists()) {
            continue;
        }
        let Ok(Some(lock)) = try_lock(&dir) else {
            continue; // in use by another live process (or unlockable)
        };
        let built_from = std::fs::read_to_string(&metadata)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v["image_id"].as_str().map(str::to_string));
        let resumable = built_from.as_deref() == Some(image_id) && disk.is_file();
        if resumable && adopted.is_none() {
            adopted = Some((id, lock));
        } else {
            let _ = std::fs::remove_dir_all(&dir);
            drop(lock);
        }
    }
    adopted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dir: &Path, image: Option<&str>) -> ComputerId {
        let id = ComputerId::new();
        let d = dir.join(id.to_string());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("disk.img"), b"disk").unwrap();
        let meta = match image {
            Some(i) => serde_json::json!({"computer_id": id.to_string(), "image_id": i}),
            None => serde_json::json!({"computer_id": id.to_string()}),
        };
        std::fs::write(d.join("metadata.json"), meta.to_string()).unwrap();
        id
    }

    #[test]
    fn adopts_current_image_and_removes_stale_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let keep = make(tmp.path(), Some("pegoles-base-0.3"));
        let old = make(tmp.path(), Some("pegoles-base-0.2"));
        let legacy = make(tmp.path(), None);
        std::fs::create_dir_all(tmp.path().join("not-a-computer")).unwrap();
        let (id, _lock) = claim_existing(tmp.path(), "pegoles-base-0.3", "disk.img").unwrap();
        assert_eq!(id, keep);
        assert!(!tmp.path().join(old.to_string()).exists());
        assert!(!tmp.path().join(legacy.to_string()).exists());
        assert!(
            tmp.path().join("not-a-computer").exists(),
            "unknown dirs untouched"
        );
    }

    #[test]
    fn a_computer_held_by_another_owner_is_never_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let live = make(tmp.path(), Some("pegoles-base-0.3"));
        let _held = lock_new(&tmp.path().join(live.to_string())).unwrap();
        assert!(claim_existing(tmp.path(), "pegoles-base-0.3", "disk.img").is_none());
        let stale_but_live = make(tmp.path(), Some("old"));
        let _held2 = lock_new(&tmp.path().join(stale_but_live.to_string())).unwrap();
        let _ = claim_existing(tmp.path(), "pegoles-base-0.3", "disk.img");
        assert!(tmp.path().join(stale_but_live.to_string()).exists());
    }

    #[test]
    fn only_one_computer_is_adopted() {
        let tmp = tempfile::tempdir().unwrap();
        make(tmp.path(), Some("img"));
        make(tmp.path(), Some("img"));
        let (_id, _lock) = claim_existing(tmp.path(), "img", "disk.img").unwrap();
        let left = std::fs::read_dir(tmp.path()).unwrap().count();
        assert_eq!(left, 1, "the duplicate is removed");
    }
}

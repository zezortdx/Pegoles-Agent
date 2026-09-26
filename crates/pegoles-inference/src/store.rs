//! Model storage outside Git: `<data>/models/<id>/`.
//!
//! ```text
//! models/
//!   <id>/                 installed: exactly the catalog files + manifest
//!     pegoles-model.json  written last, inside staging, before the rename
//!   .staging/<id>/        downloads in progress (`.part` files resume)
//!   .trash/               removed/replaced models, deleted best-effort
//! ```
//!
//! Install is atomic from the store's point of view: files are fetched
//! and hash-verified in staging, then the whole directory is renamed into
//! place. A partial or corrupted download is never visible as installed.
//! Loading re-verifies every byte against the compiled-in catalog.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::catalog::{ModelFile, ModelSpec};

pub const MANIFEST_NAME: &str = "pegoles-model.json";
/// Free space kept beyond the download itself (VM clones grow too).
pub const DISK_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("not enough disk space: {needed_bytes} bytes needed, {free_bytes} free")]
    DiskSpace { needed_bytes: u64, free_bytes: u64 },
    #[error("download failed: {0}")]
    Network(String),
    #[error("downloaded file {path} is corrupted (checksum mismatch)")]
    Corrupted { path: String },
    #[error("model {0} is not installed")]
    NotInstalled(String),
    #[error("model {id} failed verification: {reason}")]
    Invalid { id: String, reason: String },
    #[error("model {0} cannot be downloaded (no public source)")]
    NotDownloadable(String),
    #[error("cancelled")]
    Cancelled,
    #[error("storage error: {0}")]
    Io(String),
}

fn io(e: std::io::Error) -> StoreError {
    StoreError::Io(e.to_string())
}

/// Written into an installed model directory; mirrors the catalog entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledManifest {
    pub manifest_version: u32,
    pub spec: ModelSpec,
    pub installed_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallState {
    NotInstalled,
    /// Staging holds `bytes` of `total` (a resumable download).
    Partial {
        bytes: u64,
        total: u64,
    },
    Installed,
    /// Present but not matching the catalog (sizes, extra/missing files).
    Invalid {
        reason: String,
    },
}

/// A model directory whose bytes matched the catalog at verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedModel {
    pub spec: ModelSpec,
    pub dir: PathBuf,
    pub verify_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Downloading,
    Verifying,
    Finalizing,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InstallProgress {
    pub phase: InstallPhase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

/// Where bytes come from. HTTPS in production; fakes in tests.
pub trait Fetcher: Send + Sync {
    /// Stream `file` starting at byte `offset` into `sink`. Returns once
    /// the source is exhausted. `sink` returns false to stop early.
    fn fetch(
        &self,
        spec: &ModelSpec,
        file: &ModelFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), StoreError>;
}

pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn staging_dir(&self, id: &str) -> PathBuf {
        self.root.join(".staging").join(id)
    }

    /// Cheap status (sizes only, no hashing).
    pub fn state(&self, spec: &ModelSpec) -> InstallState {
        let dir = self.model_dir(&spec.id);
        if dir.exists() {
            return match self.check_layout(spec, &dir) {
                Ok(()) => InstallState::Installed,
                Err(reason) => InstallState::Invalid { reason },
            };
        }
        let staging = self.staging_dir(&spec.id);
        if staging.exists() {
            let bytes = spec
                .files
                .iter()
                .map(|f| {
                    let done = staging.join(&f.path);
                    let part = part_path(&staging, &f.path);
                    file_len(&done).or_else(|| file_len(&part)).unwrap_or(0)
                })
                .sum();
            return InstallState::Partial {
                bytes,
                total: spec.total_bytes(),
            };
        }
        InstallState::NotInstalled
    }

    /// Layout check: exactly the catalog files (regular files, right
    /// sizes) plus the manifest; nothing else, no symlinks.
    fn check_layout(&self, spec: &ModelSpec, dir: &Path) -> Result<(), String> {
        let meta = fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        if !meta.is_dir() {
            return Err("model path is not a directory".into());
        }
        let manifest: InstalledManifest = fs::read_to_string(dir.join(MANIFEST_NAME))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .ok_or("manifest missing or unreadable")?;
        if manifest.spec != *spec {
            return Err("installed manifest does not match this Pegoles version".into());
        }
        let mut expected: std::collections::HashSet<PathBuf> =
            spec.files.iter().map(|f| PathBuf::from(&f.path)).collect();
        expected.insert(PathBuf::from(MANIFEST_NAME));
        let mut found = Vec::new();
        walk(dir, dir, &mut found)?;
        for rel in &found {
            if !expected.contains(rel) {
                return Err(format!("unexpected file {}", rel.display()));
            }
        }
        for f in &spec.files {
            let p = dir.join(&f.path);
            let meta = fs::symlink_metadata(&p).map_err(|_| format!("missing {}", f.path))?;
            if !meta.file_type().is_file() {
                return Err(format!("{} is not a regular file", f.path));
            }
            if meta.len() != f.size {
                return Err(format!("{} has the wrong size", f.path));
            }
        }
        Ok(())
    }

    /// Full verification (every byte hashed) before a model is loaded.
    pub fn verify(&self, spec: &ModelSpec) -> Result<VerifiedModel, StoreError> {
        let started = std::time::Instant::now();
        let dir = self.model_dir(&spec.id);
        if !dir.exists() {
            return Err(StoreError::NotInstalled(spec.id.clone()));
        }
        self.check_layout(spec, &dir)
            .map_err(|reason| StoreError::Invalid {
                id: spec.id.clone(),
                reason,
            })?;
        for f in &spec.files {
            let digest = sha256_file(&dir.join(&f.path)).map_err(io)?;
            if !digest.eq_ignore_ascii_case(&f.sha256) {
                return Err(StoreError::Invalid {
                    id: spec.id.clone(),
                    reason: format!("{} checksum mismatch", f.path),
                });
            }
        }
        for f in spec.files.iter().filter(|f| f.path.ends_with(".json")) {
            check_json_is_inert(&dir.join(&f.path)).map_err(|reason| StoreError::Invalid {
                id: spec.id.clone(),
                reason: format!("{}: {reason}", f.path),
            })?;
        }
        Ok(VerifiedModel {
            spec: spec.clone(),
            dir,
            verify_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Download (resuming), verify, and atomically install `spec`.
    pub fn install(
        &self,
        spec: &ModelSpec,
        fetcher: &dyn Fetcher,
        progress: &mut dyn FnMut(&InstallProgress),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<VerifiedModel, StoreError> {
        spec.validate().map_err(|reason| StoreError::Invalid {
            id: spec.id.clone(),
            reason,
        })?;
        if !spec.downloadable() {
            return Err(StoreError::NotDownloadable(spec.id.clone()));
        }
        let staging = self.staging_dir(&spec.id);
        fs::create_dir_all(&staging).map_err(io)?;
        let total = spec.total_bytes();
        let mut done_before: u64 = 0;
        for f in &spec.files {
            let have = file_len(&staging.join(&f.path))
                .or_else(|| file_len(&part_path(&staging, &f.path)))
                .unwrap_or(0);
            done_before += have.min(f.size);
        }
        if let Some(free) = free_disk_bytes(&self.root) {
            let needed = total.saturating_sub(done_before) + DISK_HEADROOM_BYTES;
            if free < needed {
                return Err(StoreError::DiskSpace {
                    needed_bytes: needed,
                    free_bytes: free,
                });
            }
        }
        let mut done = done_before;
        for f in &spec.files {
            let final_path = staging.join(&f.path);
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            if file_len(&final_path) == Some(f.size) {
                continue; // fetched and verified in an earlier attempt
            }
            let part = part_path(&staging, &f.path);
            let before = file_len(&part).unwrap_or(0);
            let before = if before > f.size {
                let _ = fs::remove_file(&part);
                0
            } else {
                before
            };
            done -= before.min(done);
            let digest = fetch_file(
                spec,
                f,
                &part,
                fetcher,
                &mut |n| {
                    progress(&InstallProgress {
                        phase: InstallPhase::Downloading,
                        done_bytes: done + n,
                        total_bytes: total,
                        file: f.path.clone(),
                    });
                },
                cancelled,
            )?;
            progress(&InstallProgress {
                phase: InstallPhase::Verifying,
                done_bytes: done + f.size,
                total_bytes: total,
                file: f.path.clone(),
            });
            if !digest.eq_ignore_ascii_case(&f.sha256) {
                // Corrupted or tampered: never keep it, never resume it.
                let _ = fs::remove_file(&part);
                return Err(StoreError::Corrupted {
                    path: f.path.clone(),
                });
            }
            fs::rename(&part, &final_path).map_err(io)?;
            done += f.size;
        }
        progress(&InstallProgress {
            phase: InstallPhase::Finalizing,
            done_bytes: total,
            total_bytes: total,
            file: String::new(),
        });
        self.finalize(spec, &staging)?;
        self.verify(spec)
    }

    /// Import a model produced locally (e.g. a conversion) from `src`,
    /// accepting it only if every byte matches the catalog entry.
    pub fn import_local(&self, spec: &ModelSpec, src: &Path) -> Result<VerifiedModel, StoreError> {
        spec.validate().map_err(|reason| StoreError::Invalid {
            id: spec.id.clone(),
            reason,
        })?;
        let staging = self.staging_dir(&spec.id);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io)?;
        }
        fs::create_dir_all(&staging).map_err(io)?;
        for f in &spec.files {
            let from = src.join(&f.path);
            let meta = fs::symlink_metadata(&from).map_err(|_| StoreError::Invalid {
                id: spec.id.clone(),
                reason: format!("{} missing in import source", f.path),
            })?;
            if !meta.file_type().is_file() || meta.len() != f.size {
                return Err(StoreError::Invalid {
                    id: spec.id.clone(),
                    reason: format!("{} has the wrong size or type", f.path),
                });
            }
            let to = staging.join(&f.path);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            // APFS clone when possible (no extra space), else a copy.
            if clone_file(&from, &to).is_err() {
                fs::copy(&from, &to).map_err(io)?;
            }
            if !sha256_file(&to)
                .map_err(io)?
                .eq_ignore_ascii_case(&f.sha256)
            {
                let _ = fs::remove_dir_all(&staging);
                return Err(StoreError::Corrupted {
                    path: f.path.clone(),
                });
            }
        }
        self.finalize(spec, &staging)?;
        self.verify(spec)
    }

    fn finalize(&self, spec: &ModelSpec, staging: &Path) -> Result<(), StoreError> {
        let manifest = InstalledManifest {
            manifest_version: 1,
            spec: spec.clone(),
            installed_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let raw =
            serde_json::to_vec_pretty(&manifest).map_err(|e| StoreError::Io(e.to_string()))?;
        let mpath = staging.join(MANIFEST_NAME);
        let mut out = fs::File::create(&mpath).map_err(io)?;
        out.write_all(&raw).map_err(io)?;
        out.sync_all().map_err(io)?;
        let dest = self.model_dir(&spec.id);
        if dest.exists() {
            self.trash(&dest)?;
        }
        fs::rename(staging, &dest).map_err(io)?;
        Ok(())
    }

    fn trash(&self, dir: &Path) -> Result<(), StoreError> {
        let trash = self.root.join(".trash");
        fs::create_dir_all(&trash).map_err(io)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let to = trash.join(format!("{name}-{stamp}"));
        fs::rename(dir, &to).map_err(io)?;
        let _ = fs::remove_dir_all(&to);
        Ok(())
    }

    /// Remove an installed model and any staged download for it.
    pub fn remove(&self, id: &str) -> Result<(), StoreError> {
        crate::catalog::validate_rel_path(id).map_err(StoreError::Io)?;
        let dir = self.model_dir(id);
        if dir.exists() {
            self.trash(&dir)?;
        }
        let staging = self.staging_dir(id);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io)?;
        }
        Ok(())
    }

    /// Discard a staged (partial) download.
    pub fn discard_partial(&self, id: &str) -> Result<(), StoreError> {
        crate::catalog::validate_rel_path(id).map_err(StoreError::Io)?;
        let staging = self.staging_dir(id);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io)?;
        }
        Ok(())
    }
}

fn part_path(staging: &Path, rel: &str) -> PathBuf {
    staging.join(format!("{rel}.part"))
}

fn file_len(p: &Path) -> Option<u64> {
    fs::symlink_metadata(p)
        .ok()
        .filter(|m| m.file_type().is_file())
        .map(|m| m.len())
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .map_err(|e| e.to_string())?
            .to_path_buf();
        if ty.is_symlink() {
            return Err(format!("symlink {} in model directory", rel.display()));
        }
        if ty.is_dir() {
            walk(base, &path, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// Any model JSON (config, processor, tokenizer…) that asks the loader
/// for remote code is refused even when its bytes are pinned. This is
/// defense in depth: the worker also disables transformers' dynamic
/// module loading and loads with `trust_remote_code=False`.
fn check_json_is_inert(path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    for key in ["auto_map", "custom_pipelines"] {
        if v.get(key).is_some() {
            return Err(format!("requests remote code ({key})"));
        }
    }
    Ok(())
}

/// SHA-256 via `ring` (hardware-accelerated on Apple silicon: ~3x the
/// portable implementation, which matters for multi-GB weights).
struct Sha256(ring::digest::Context);

impl Sha256 {
    fn new() -> Self {
        Self(ring::digest::Context::new(&ring::digest::SHA256))
    }
    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
    fn finalize(self) -> impl AsRef<[u8]> {
        self.0.finish()
    }
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Fetch into `part` (resuming from its current length) and return the
/// SHA-256 of the complete file.
fn fetch_file(
    spec: &ModelSpec,
    f: &ModelFile,
    part: &Path,
    fetcher: &dyn Fetcher,
    progress: &mut dyn FnMut(u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<String, StoreError> {
    let mut hasher = Sha256::new();
    let mut have: u64 = 0;
    if let Ok(mut existing) = fs::File::open(part) {
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = existing.read(&mut buf).map_err(io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }
    if have < f.size {
        let mut out = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(part)
            .map_err(io)?;
        let mut write_err = None;
        let mut overflow = false;
        let mut stopped = false;
        fetcher.fetch(spec, f, have, &mut |chunk| {
            if cancelled() {
                stopped = true;
                return false;
            }
            if have + chunk.len() as u64 > f.size {
                overflow = true;
                return false;
            }
            if let Err(e) = out.write_all(chunk) {
                write_err = Some(e);
                return false;
            }
            hasher.update(chunk);
            have += chunk.len() as u64;
            progress(have);
            true
        })?;
        out.sync_all().map_err(io)?;
        if let Some(e) = write_err {
            return Err(io(e));
        }
        if stopped {
            return Err(StoreError::Cancelled);
        }
        if overflow {
            drop(out);
            let _ = fs::remove_file(part);
            return Err(StoreError::Corrupted {
                path: f.path.clone(),
            });
        }
    }
    if have != f.size {
        return Err(StoreError::Network(format!(
            "{} ended early ({have} of {} bytes); retry to resume",
            f.path, f.size
        )));
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(unix)]
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
    // SAFETY: zeroed statvfs out-param, valid C path.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    (rc == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
pub fn free_disk_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn clone_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> i32;
    }
    let a = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let b = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: two valid C paths; flag CLONE_NOFOLLOW (1).
    if unsafe { clonefile(a.as_ptr(), b.as_ptr(), 1) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn clone_file(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("clonefile unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{ModelFamily, ModelSource};
    use sha2::Digest;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn sha(b: &[u8]) -> String {
        hex::encode(sha2::Sha256::digest(b))
    }

    fn spec(files: &[(&str, &[u8])]) -> ModelSpec {
        ModelSpec {
            id: "test-model".into(),
            display_name: "Test".into(),
            family: ModelFamily::Qwen3Vl,
            architecture: "qwen3_vl".into(),
            parameters: "2B".into(),
            quantization: "4bit".into(),
            source: ModelSource::Huggingface {
                repo: "org/repo".into(),
                revision: "0123456789abcdef0123456789abcdef01234567".into(),
            },
            license: "apache-2.0".into(),
            pegoles_min_version: "0.1.0".into(),
            recommended_min_ram_gb: None,
            published: "2026-01-01".into(),
            files: files
                .iter()
                .map(|(p, b)| ModelFile {
                    path: p.to_string(),
                    size: b.len() as u64,
                    sha256: sha(b),
                })
                .collect(),
        }
    }

    /// Serves bytes from memory; can cut a file short, corrupt it, or
    /// record the offsets it was asked for (resume proof).
    #[derive(Default)]
    struct FakeFetcher {
        files: HashMap<String, Vec<u8>>,
        cut_after: Mutex<Option<usize>>,
        offsets: Mutex<Vec<u64>>,
    }

    impl Fetcher for FakeFetcher {
        fn fetch(
            &self,
            _spec: &ModelSpec,
            file: &ModelFile,
            offset: u64,
            sink: &mut dyn FnMut(&[u8]) -> bool,
        ) -> Result<(), StoreError> {
            self.offsets.lock().unwrap().push(offset);
            let data = &self.files[&file.path][offset as usize..];
            let cut = self.cut_after.lock().unwrap().take();
            let data = match cut {
                Some(n) => &data[..n.min(data.len())],
                None => data,
            };
            for chunk in data.chunks(3) {
                if !sink(chunk) {
                    break;
                }
            }
            if cut.is_some() {
                return Err(StoreError::Network("connection reset".into()));
            }
            Ok(())
        }
    }

    fn cfg() -> &'static [u8] {
        br#"{"model_type":"qwen3_vl"}"#
    }

    #[test]
    fn install_resumes_verifies_and_is_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let weights = b"0123456789abcdefghij".to_vec();
        let s = spec(&[("config.json", cfg()), ("model.safetensors", &weights)]);
        let fetcher = FakeFetcher {
            files: HashMap::from([
                ("config.json".to_string(), cfg().to_vec()),
                ("model.safetensors".to_string(), weights.clone()),
            ]),
            cut_after: Mutex::new(None),
            offsets: Mutex::new(vec![]),
        };
        assert_eq!(store.state(&s), InstallState::NotInstalled);

        // First attempt: config ok, weights cut after 7 bytes.
        let mut first = true;
        let res = store.install(
            &s,
            &FetchCutOnce {
                inner: &fetcher,
                first: Mutex::new(&mut first),
            },
            &mut |_| {},
            &|| false,
        );
        assert!(matches!(res, Err(StoreError::Network(_))));
        assert!(matches!(store.state(&s), InstallState::Partial { .. }));
        assert!(!tmp.path().join("models/test-model").exists());

        // Second attempt resumes at byte 7 of the weights.
        fetcher.offsets.lock().unwrap().clear();
        let v = store.install(&s, &fetcher, &mut |_| {}, &|| false).unwrap();
        assert_eq!(*fetcher.offsets.lock().unwrap(), vec![7]);
        assert_eq!(store.state(&s), InstallState::Installed);
        assert!(v.dir.join("model.safetensors").exists());
        assert!(store.verify(&s).is_ok());
    }

    struct FetchCutOnce<'a> {
        inner: &'a FakeFetcher,
        first: Mutex<&'a mut bool>,
    }
    impl Fetcher for FetchCutOnce<'_> {
        fn fetch(
            &self,
            spec: &ModelSpec,
            file: &ModelFile,
            offset: u64,
            sink: &mut dyn FnMut(&[u8]) -> bool,
        ) -> Result<(), StoreError> {
            if file.path == "model.safetensors" {
                let mut first = self.first.lock().unwrap();
                if **first {
                    **first = false;
                    *self.inner.cut_after.lock().unwrap() = Some(7);
                }
            }
            self.inner.fetch(spec, file, offset, sink)
        }
    }

    #[test]
    fn corrupted_download_is_discarded_never_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let s = spec(&[("config.json", cfg()), ("model.safetensors", b"good-bytes")]);
        let fetcher = FakeFetcher {
            files: HashMap::from([
                ("config.json".to_string(), cfg().to_vec()),
                ("model.safetensors".to_string(), b"evil-bytes".to_vec()),
            ]),
            ..Default::default()
        };
        let res = store.install(&s, &fetcher, &mut |_| {}, &|| false);
        assert!(matches!(res, Err(StoreError::Corrupted { .. })));
        assert_ne!(store.state(&s), InstallState::Installed);
        assert!(!tmp.path().join("models/test-model").exists());
        // The corrupt part file was deleted: a retry starts from zero.
        assert!(!tmp
            .path()
            .join("models/.staging/test-model/model.safetensors.part")
            .exists());
    }

    #[test]
    fn oversized_stream_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let s = spec(&[("config.json", cfg()), ("model.safetensors", b"abc")]);
        let fetcher = FakeFetcher {
            files: HashMap::from([
                ("config.json".to_string(), cfg().to_vec()),
                ("model.safetensors".to_string(), b"abcdefgh".to_vec()),
            ]),
            ..Default::default()
        };
        let res = store.install(&s, &fetcher, &mut |_| {}, &|| false);
        assert!(matches!(res, Err(StoreError::Corrupted { .. })));
    }

    #[test]
    fn cancellation_keeps_a_resumable_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let s = spec(&[("config.json", cfg())]);
        let fetcher = FakeFetcher {
            files: HashMap::from([("config.json".to_string(), cfg().to_vec())]),
            ..Default::default()
        };
        let res = store.install(&s, &fetcher, &mut |_| {}, &|| true);
        assert_eq!(res, Err(StoreError::Cancelled));
        assert!(matches!(store.state(&s), InstallState::Partial { .. }));
    }

    #[test]
    fn tampering_after_install_fails_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let s = spec(&[("config.json", cfg()), ("model.safetensors", b"weights!")]);
        let fetcher = FakeFetcher {
            files: HashMap::from([
                ("config.json".to_string(), cfg().to_vec()),
                ("model.safetensors".to_string(), b"weights!".to_vec()),
            ]),
            ..Default::default()
        };
        let v = store.install(&s, &fetcher, &mut |_| {}, &|| false).unwrap();
        // Same size, different bytes: only hashing catches it.
        fs::write(v.dir.join("model.safetensors"), b"WEIGHTS!").unwrap();
        assert!(matches!(store.verify(&s), Err(StoreError::Invalid { .. })));
        // An extra file (e.g. dropped-in Python) is refused.
        fs::write(v.dir.join("model.safetensors"), b"weights!").unwrap();
        fs::write(v.dir.join("modeling_evil.py"), b"import os").unwrap();
        assert!(matches!(store.state(&s), InstallState::Invalid { .. }));
        fs::remove_file(v.dir.join("modeling_evil.py")).unwrap();
        // A symlinked weight file is refused.
        fs::remove_file(v.dir.join("model.safetensors")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", v.dir.join("model.safetensors")).unwrap();
            assert!(matches!(store.state(&s), InstallState::Invalid { .. }));
        }
        store.remove(&s.id).unwrap();
        assert_eq!(store.state(&s), InstallState::NotInstalled);
    }

    #[test]
    fn remote_code_in_any_model_json_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let evil = br#"{"processor_class":"X","auto_map":{"AutoProcessor":"p.X"}}"#;
        let s = spec(&[("config.json", cfg()), ("processor_config.json", evil)]);
        let fetcher = FakeFetcher {
            files: HashMap::from([
                ("config.json".to_string(), cfg().to_vec()),
                ("processor_config.json".to_string(), evil.to_vec()),
            ]),
            ..Default::default()
        };
        assert!(matches!(
            store.install(&s, &fetcher, &mut |_| {}, &|| false),
            Err(StoreError::Invalid { .. })
        ));
    }

    #[test]
    fn remote_code_configs_are_refused_even_when_pinned() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let evil = br#"{"model_type":"x","auto_map":{"AutoModel":"modeling_x.Model"}}"#;
        let s = spec(&[("config.json", evil)]);
        let fetcher = FakeFetcher {
            files: HashMap::from([("config.json".to_string(), evil.to_vec())]),
            ..Default::default()
        };
        let res = store.install(&s, &fetcher, &mut |_| {}, &|| false);
        assert!(matches!(res, Err(StoreError::Invalid { .. })));
    }

    #[test]
    fn local_import_requires_matching_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("conv");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("config.json"), cfg()).unwrap();
        fs::write(src.join("model.safetensors"), b"converted").unwrap();
        let store = ModelStore::new(tmp.path().join("models"));
        let s = spec(&[("config.json", cfg()), ("model.safetensors", b"converted")]);
        assert!(store.import_local(&s, &src).is_ok());
        let wrong = spec(&[("config.json", cfg()), ("model.safetensors", b"convertex")]);
        store.remove(&s.id).unwrap();
        assert!(store.import_local(&wrong, &src).is_err());
        assert_ne!(store.state(&wrong), InstallState::Installed);
    }
}

//! Distribution of the sealed Pegoles guest image.
//!
//! A release boots exactly one image, pinned in `catalog/images.json`, which
//! is compiled into the (signed, notarized) app: the digest of the
//! downloadable archive (SHA-256) and of the disk it unpacks to (SHA-512),
//! with their sizes. Nothing downloaded is trusted by name, URL or file
//! presence:
//!
//! ```text
//! HTTPS (resumable) ─▶ .staging/<id>/archive.gz.part ─▶ size + SHA-256 == pin
//!   ─▶ gzip (pure Rust, output bounded to the pinned size, sparse writes)
//!   ─▶ disk.raw.part: size + SHA-512 == pin ─▶ manifest + marker from the pin
//!   ─▶ atomic rename to images/<id> (an older copy goes to .trash first)
//! ```
//!
//! Before a release clones the image for a computer it re-hashes the whole
//! disk once per app session ([`verify_installed`]) and refuses any mismatch;
//! the manifest on disk is never the source of truth for a release.

use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};

use crate::error::{ComputerError, Result};
use crate::image::{ArtifactRecord, DerivedManifest, GraphicalImageInfo};
use crate::platform::DiskFormat;

const CATALOG_JSON: &str = include_str!("../catalog/images.json");
/// Largest disk / archive a catalog entry may declare (sanity bounds).
const MAX_DISK_BYTES: u64 = 64 << 30;
const MAX_ARCHIVE_BYTES: u64 = 8 << 30;
/// Free space kept in reserve beyond the worst case of the install.
const SPACE_MARGIN_BYTES: u64 = 512 << 20;
const CHUNK: usize = 1 << 20;
/// Placeholder a catalog carries until the release host is decided.
pub const UNPUBLISHED_MARKER: &str = "UNPUBLISHED";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseImage {
    pub id: String,
    pub version: String,
    pub debian_version: String,
    pub architecture: String,
    pub guest_runtime_version: String,
    pub guest_protocol_version: u32,
    pub source_image_sha512: String,
    pub built_at: String,
    pub graphical: GraphicalImageInfo,
    pub capabilities: Vec<String>,
    pub disk: ReleaseDisk,
    pub archive: ReleaseArchive,
    /// Where the image comes from and how it was built (for people and
    /// the release manifest; not interpreted by the installer).
    pub provenance: ImageProvenance,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseDisk {
    pub file_name: String,
    pub bytes: u64,
    pub sha512: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseArchive {
    pub file_name: String,
    pub compression: String,
    pub bytes: u64,
    pub sha256: String,
    /// HTTPS mirrors, tried in order. All serve the same pinned bytes.
    pub urls: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageProvenance {
    pub build: String,
    pub packages_manifest: String,
    pub packages_manifest_sha256: String,
    pub sanitized: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: u32,
    images: Vec<ReleaseImage>,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl ReleaseImage {
    fn validate(&self) -> Result<()> {
        let bad = |why: &str| {
            Err(ComputerError::ImageVerificationFailed(format!(
                "catalog entry {}: {why}",
                self.id
            )))
        };
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && !self.id.starts_with('.');
        if !id_ok {
            return bad("invalid id");
        }
        let archive_name_ok = !self.archive.file_name.is_empty()
            && self.archive.file_name.len() <= 128
            && !self.archive.file_name.starts_with('.')
            && self
                .archive
                .file_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && self.archive.file_name.ends_with(".gz");
        if !archive_name_ok {
            return bad("invalid archive file name");
        }
        if self.disk.file_name != "disk.raw" {
            return bad("disk file must be disk.raw");
        }
        if self.disk.bytes == 0
            || self.disk.bytes > MAX_DISK_BYTES
            || !self.disk.bytes.is_multiple_of(512)
        {
            return bad("disk size out of bounds");
        }
        if !is_hex(&self.disk.sha512, 128) || !is_hex(&self.archive.sha256, 64) {
            return bad("digests must be lowercase hex SHA-512 / SHA-256");
        }
        if self.archive.compression != "gzip" {
            return bad("only gzip archives are supported");
        }
        if self.archive.bytes == 0 || self.archive.bytes > MAX_ARCHIVE_BYTES {
            return bad("archive size out of bounds");
        }
        if self.archive.urls.is_empty()
            || self.archive.urls.iter().any(|u| !u.starts_with("https://"))
        {
            return bad("archive URLs must be HTTPS");
        }
        Ok(())
    }

    /// Whether the archive has a real download location (the catalog
    /// carries [`UNPUBLISHED_MARKER`] until the release host exists).
    pub fn is_published(&self) -> bool {
        !self
            .archive
            .urls
            .iter()
            .any(|u| u.contains(UNPUBLISHED_MARKER))
    }

    /// The manifest an installed copy of this image carries (written from
    /// the pin, never from downloaded data).
    pub fn manifest(&self) -> DerivedManifest {
        DerivedManifest {
            image_id: self.id.clone(),
            pegoles_image_version: self.version.clone(),
            debian_version: self.debian_version.clone(),
            architecture: self.architecture.clone(),
            guest_runtime_version: self.guest_runtime_version.clone(),
            guest_protocol_version: self.guest_protocol_version,
            source_image_sha512: self.source_image_sha512.clone(),
            image_sha512: self.disk.sha512.clone(),
            built_at: self.built_at.clone(),
            artifacts: vec![ArtifactRecord {
                file_name: self.disk.file_name.clone(),
                disk_format: DiskFormat::Raw,
                sha512: self.disk.sha512.clone(),
                bytes: self.disk.bytes,
                architecture: self.architecture.clone(),
                guest_runtime_version: self.guest_runtime_version.clone(),
                source_image_sha512: self.source_image_sha512.clone(),
                built_at: self.built_at.clone(),
            }],
            graphical: Some(self.graphical.clone()),
            capabilities: self.capabilities.clone(),
        }
    }
}

fn parse_catalog(raw: &str) -> Result<Vec<ReleaseImage>> {
    let catalog: Catalog = serde_json::from_str(raw)
        .map_err(|e| ComputerError::ImageVerificationFailed(format!("image catalog: {e}")))?;
    if catalog.schema != 1 {
        return Err(ComputerError::ImageVerificationFailed(
            "image catalog: unknown schema".into(),
        ));
    }
    for image in &catalog.images {
        image.validate()?;
    }
    Ok(catalog.images)
}

/// The compiled-in catalog (parsed and validated once).
pub fn catalog() -> Result<&'static [ReleaseImage]> {
    static CATALOG: OnceLock<std::result::Result<Vec<ReleaseImage>, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| parse_catalog(CATALOG_JSON).map_err(|e| e.to_string()))
        .as_deref()
        .map_err(|e| ComputerError::ImageVerificationFailed(e.clone()))
}

/// The pinned release entry for an image id, if any.
pub fn release_image(id: &str) -> Option<&'static ReleaseImage> {
    catalog().ok()?.iter().find(|i| i.id == id)
}

/// Whether pins are enforced for boot. Release builds always; debug builds
/// accept a locally sealed, self-consistent image (the image-builder loop
/// reseals with new digests). This crate's own unit tests seed small fake
/// images in both profiles, so they exercise enforcement explicitly
/// (`derived_status_with`, `instantiate_boot_source_with`) instead.
pub fn pins_enforced() -> bool {
    !cfg!(debug_assertions) && !cfg!(test)
}

/// Cheap consistency check of an installed image against its pin (no
/// hashing): manifest digests and sizes, and a regular disk file of the
/// pinned size. Used for status; [`verify_installed`] hashes the bytes.
pub fn installed_matches_pin(dir: &Path, image: &ReleaseImage) -> bool {
    let manifest: Option<DerivedManifest> = fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let Some(manifest) = manifest else {
        return false;
    };
    let record_ok = manifest.artifacts.len() == 1
        && manifest.artifacts[0].disk_format == DiskFormat::Raw
        && manifest.artifacts.first().is_some_and(|a| {
            a.file_name == image.disk.file_name
                && a.sha512 == image.disk.sha512
                && a.bytes == image.disk.bytes
        });
    let disk_ok = fs::symlink_metadata(dir.join(&image.disk.file_name))
        .is_ok_and(|m| m.file_type().is_file() && m.len() == image.disk.bytes);
    manifest.image_id == image.id
        && manifest.image_sha512 == image.disk.sha512
        && record_ok
        && disk_ok
}

/// Identity of a file for the once-per-session verification cache.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
}

fn stamp(path: &Path) -> Result<FileStamp> {
    let m = fs::symlink_metadata(path)
        .map_err(|e| ComputerError::ImageMissing(format!("{}: {e}", path.display())))?;
    if !m.file_type().is_file() {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    let (dev, ino, mtime_ns) = {
        use std::os::unix::fs::MetadataExt;
        (
            m.dev(),
            m.ino(),
            i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec()),
        )
    };
    #[cfg(not(unix))]
    let (dev, ino, mtime_ns) = (
        0,
        0,
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as i128),
    );
    Ok(FileStamp {
        dev,
        ino,
        len: m.len(),
        mtime_ns,
    })
}

/// Hash the installed disk against its pin, once per session per file
/// identity (a replaced or modified file is hashed again). Fails closed.
pub fn verify_installed(dir: &Path, image: &ReleaseImage) -> Result<()> {
    static VERIFIED: OnceLock<Mutex<HashMap<PathBuf, FileStamp>>> = OnceLock::new();
    if !installed_matches_pin(dir, image) {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "installed image {} does not match the pinned release image",
            image.id
        )));
    }
    let disk = dir.join(&image.disk.file_name);
    let before = stamp(&disk)?;
    let cache = VERIFIED.get_or_init(|| Mutex::new(HashMap::new()));
    if cache.lock().unwrap_or_else(|e| e.into_inner()).get(&disk) == Some(&before) {
        return Ok(());
    }
    let digest = sha512_of(&disk, &mut |_, _| {}, &|| false)?;
    if digest != image.disk.sha512 || stamp(&disk)? != before {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "image {} bytes do not match the pinned SHA-512",
            image.id
        )));
    }
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(disk, before);
    Ok(())
}

fn sha512_of(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| ComputerError::Backend(e.to_string()))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(CHUNK, file);
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        if cancel() {
            return Err(cancelled());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn sha256_of(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| ComputerError::Backend(e.to_string()))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(CHUNK, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        if cancel() {
            return Err(cancelled());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn cancelled() -> ComputerError {
    ComputerError::Backend("image installation cancelled".into())
}

/// Install stages reported to the progress callback (real bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStage {
    Downloading,
    VerifyingArchive,
    Unpacking,
    Finalizing,
}

impl InstallStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallStage::Downloading => "downloading",
            InstallStage::VerifyingArchive => "verifying",
            InstallStage::Unpacking => "unpacking",
            InstallStage::Finalizing => "finalizing",
        }
    }
}

/// Byte source for the archive (HTTPS in production, a fake in tests).
pub trait ArchiveSource {
    /// Open `url` at byte `offset`. Returns the body and whether it starts
    /// at `offset` (a ranged response) or at 0 (the whole file).
    fn open(&self, url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool)>;
}

/// HTTPS-only source: TLS with the bundled roots, bounded redirects that can
/// never leave HTTPS, ranged requests for resume.
pub struct HttpsSource {
    agent: ureq::Agent,
}

/// Bytes requested per HTTPS request: every request is short and bounded
/// by the timeouts below, so a stalled host costs at most a minute and a
/// cancel is honoured at least that often; progress is kept between them.
const RANGE_BYTES: u64 = 8 << 20;

impl Default for HttpsSource {
    fn default() -> Self {
        use std::time::Duration;
        Self {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .https_only(true)
                    .max_redirects(5)
                    .timeout_connect(Some(Duration::from_secs(30)))
                    .timeout_recv_response(Some(Duration::from_secs(30)))
                    .timeout_recv_body(Some(Duration::from_secs(60)))
                    .build(),
            ),
        }
    }
}

impl ArchiveSource for HttpsSource {
    fn open(&self, url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool)> {
        if !url.starts_with("https://") {
            return Err(ComputerError::ImageVerificationFailed(
                "image URLs must be HTTPS".into(),
            ));
        }
        let end = offset.saturating_add(RANGE_BYTES - 1);
        let req = self
            .agent
            .get(url)
            .header("Range", &format!("bytes={offset}-{end}"));
        let res = req
            .call()
            .map_err(|e| ComputerError::Backend(format!("image download failed: {e}")))?;
        let partial = res.status().as_u16() == 206;
        Ok((Box::new(res.into_body().into_reader()), partial))
    }
}

/// Serves a local copy of the published archive (release engineering and
/// development). The pins are unchanged: every byte is still checked.
pub struct LocalArchiveSource(pub PathBuf);

impl ArchiveSource for LocalArchiveSource {
    fn open(&self, _url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool)> {
        let mut f = fs::File::open(&self.0).map_err(io_err)?;
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        Ok((Box::new(f), offset > 0))
    }
}

/// Debug builds only: `PEGOLES_IMAGE_ARCHIVE=<local .raw.gz>` installs from
/// a local copy of the published archive instead of HTTPS (to exercise the
/// in-app setup before the archive is hosted). Release builds never read it.
pub fn dev_archive_override() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("PEGOLES_IMAGE_ARCHIVE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

fn io_err(e: std::io::Error) -> ComputerError {
    ComputerError::Backend(e.to_string())
}

/// Create an owner-only directory (never through a symlink).
fn private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path).map_err(io_err)?;
    let meta = fs::symlink_metadata(path).map_err(io_err)?;
    if !meta.file_type().is_dir() {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_err)?;
    }
    Ok(())
}

fn read_only(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(io_err)
    }
    #[cfg(not(unix))]
    {
        let mut perms = fs::metadata(path).map_err(io_err)?.permissions();
        perms.set_readonly(true);
        fs::set_permissions(path, perms).map_err(io_err)
    }
}

#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized,
    // zero-initialized out-parameter that statvfs fills on success.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    #[allow(clippy::unnecessary_cast)] // field widths differ across Unix targets
    (rc == 0).then(|| (st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

/// Download (resumable), verify and install a pinned release image into
/// `images_dir/<id>`. Idempotent: an interrupted install resumes from the
/// staged archive. Nothing is installed unless every byte matched its pin.
pub fn install(
    images_dir: &Path,
    image: &ReleaseImage,
    source: &dyn ArchiveSource,
    progress: &mut dyn FnMut(InstallStage, u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<PathBuf> {
    image.validate()?;
    if !image.is_published() {
        return Err(ComputerError::ImageMissing(format!(
            "no download location is configured for {} in this build",
            image.id
        )));
    }
    private_dir(images_dir)?;
    let staging_root = images_dir.join(".staging");
    private_dir(&staging_root)?;
    let staging = staging_root.join(&image.id);
    private_dir(&staging)?;

    let archive_part = staging.join(format!("{}.part", image.archive.file_name));
    let archive = staging.join(&image.archive.file_name);
    let already = fs::symlink_metadata(&archive).map(|m| m.len()).unwrap_or(0);
    let staged = fs::symlink_metadata(&archive_part)
        .map(|m| m.len())
        .unwrap_or(0);
    let needed = (image.archive.bytes.saturating_sub(already.max(staged)))
        + image.disk.bytes
        + SPACE_MARGIN_BYTES;
    if let Some(free) = free_bytes(images_dir) {
        if free < needed {
            return Err(ComputerError::Backend(format!(
                "not enough free disk space to install the Pegoles computer image: {} MB needed, {} MB free",
                needed >> 20,
                free >> 20
            )));
        }
    }

    // 1. Archive: download (resuming a partial file), then verify.
    if !(already == image.archive.bytes
        && fs::symlink_metadata(&archive).is_ok_and(|m| m.file_type().is_file()))
    {
        let _ = fs::remove_file(&archive);
        download(image, source, &archive_part, progress, cancel)?;
        fs::rename(&archive_part, &archive).map_err(io_err)?;
    }
    let got = sha256_of(
        &archive,
        &mut |d, t| progress(InstallStage::VerifyingArchive, d, t),
        cancel,
    )?;
    if got != image.archive.sha256 {
        let _ = fs::remove_file(&archive);
        return Err(ComputerError::ImageVerificationFailed(format!(
            "downloaded image archive for {} does not match its pinned SHA-256",
            image.id
        )));
    }

    // 2. Disk: bounded, sparse, hashed while it is written.
    let disk_part = staging.join(format!("{}.part", image.disk.file_name));
    let result = unpack(image, &archive, &disk_part, progress, cancel);
    if let Err(e) = result {
        let _ = fs::remove_file(&disk_part);
        return Err(e);
    }

    // 3. Finalize from the pin: read-only disk, manifest, marker.
    progress(InstallStage::Finalizing, 0, 1);
    let disk = staging.join(&image.disk.file_name);
    fs::rename(&disk_part, &disk).map_err(io_err)?;
    read_only(&disk)?;
    let manifest = serde_json::to_string_pretty(&image.manifest())
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
    write_synced(&staging.join("manifest.json"), manifest.as_bytes())?;
    write_synced(
        &staging.join(format!("{}.verified", image.disk.file_name)),
        image.disk.sha512.as_bytes(),
    )?;
    fs::remove_file(&archive).map_err(io_err)?;

    // 4. Atomic swap into place; an existing (older or invalid) copy is
    //    moved aside first and removed afterwards.
    let dest = images_dir.join(&image.id);
    if fs::symlink_metadata(&dest).is_ok() {
        let trash = images_dir.join(".trash");
        private_dir(&trash)?;
        let aside = trash.join(format!(
            "{}-{}",
            image.id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::rename(&dest, &aside).map_err(io_err)?;
        let _ = remove_tree(&aside);
    }
    fs::rename(&staging, &dest).map_err(io_err)?;
    progress(InstallStage::Finalizing, 1, 1);
    Ok(dest)
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    // Read-only files inside (the disk) need their directory writable only.
    fs::remove_dir_all(path)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::File::create(path).map_err(io_err)?;
    f.write_all(bytes).map_err(io_err)?;
    f.sync_all().map_err(io_err)
}

/// Consecutive attempts on one mirror that add no bytes before moving on.
const MAX_STALLED_ATTEMPTS: u32 = 3;

fn download(
    image: &ReleaseImage,
    source: &dyn ArchiveSource,
    part: &Path,
    progress: &mut dyn FnMut(InstallStage, u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<()> {
    let total = image.archive.bytes;
    let mut last_err = None;
    for url in &image.archive.urls {
        let mut stalled = 0;
        while stalled < MAX_STALLED_ATTEMPTS {
            if cancel() {
                return Err(cancelled());
            }
            let mut have = match fs::symlink_metadata(part) {
                Ok(m) if m.file_type().is_file() && m.len() <= total => m.len(),
                Ok(_) => {
                    let _ = fs::remove_file(part);
                    0
                }
                Err(_) => 0,
            };
            if have == total {
                return Ok(());
            }
            let before = have;
            let (mut body, ranged) = match source.open(url, have) {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(e);
                    stalled += 1;
                    continue;
                }
            };
            let mut out = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(part)
                .map_err(io_err)?;
            if !ranged && have > 0 {
                // The server sent the whole file: start over.
                have = 0;
                out.set_len(0).map_err(io_err)?;
            }
            out.seek(SeekFrom::Start(have)).map_err(io_err)?;
            let mut buf = vec![0u8; CHUNK];
            let outcome = loop {
                if cancel() {
                    break Err(cancelled());
                }
                let n = match body.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) => {
                        break Err(ComputerError::Backend(format!(
                            "image download interrupted: {e}"
                        )))
                    }
                };
                if n == 0 {
                    break Ok(());
                }
                if have + n as u64 > total {
                    // Never keep more bytes than the pin allows.
                    let _ = out.set_len(0);
                    break Err(ComputerError::ImageVerificationFailed(
                        "image archive is larger than its pinned size".into(),
                    ));
                }
                if let Err(e) = out.write_all(&buf[..n]) {
                    break Err(io_err(e));
                }
                have += n as u64;
                progress(InstallStage::Downloading, have, total);
            };
            out.sync_all().map_err(io_err)?;
            match outcome {
                Ok(()) if have == total => return Ok(()),
                Ok(()) => {}
                Err(e) => {
                    if matches!(e, ComputerError::ImageVerificationFailed(_)) || cancel() {
                        return Err(e);
                    }
                    last_err = Some(e);
                }
            }
            // Ranged requests end early by design; only attempts that add
            // nothing count towards giving up on this mirror.
            if have > before {
                stalled = 0;
            } else {
                stalled += 1;
            }
        }
    }
    let have = fs::symlink_metadata(part).map(|m| m.len()).unwrap_or(0);
    Err(last_err.unwrap_or_else(|| {
        ComputerError::Backend(format!(
            "image download stopped at {have} of {total} bytes; retry to resume"
        ))
    }))
}

fn unpack(
    image: &ReleaseImage,
    archive: &Path,
    disk_part: &Path,
    progress: &mut dyn FnMut(InstallStage, u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<()> {
    let total = image.disk.bytes;
    let input = fs::File::open(archive).map_err(io_err)?;
    let mut decoder = flate2::read::GzDecoder::new(BufReader::with_capacity(CHUNK, input));
    let _ = fs::remove_file(disk_part);
    let mut out = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(disk_part)
        .map_err(io_err)?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; CHUNK];
    let mut written = 0u64;
    loop {
        if cancel() {
            return Err(cancelled());
        }
        let n = decoder.read(&mut buf).map_err(|e| {
            ComputerError::ImageVerificationFailed(format!("image archive is corrupt: {e}"))
        })?;
        if n == 0 {
            break;
        }
        if written + n as u64 > total {
            return Err(ComputerError::ImageVerificationFailed(
                "image unpacks to more bytes than pinned".into(),
            ));
        }
        let chunk = &buf[..n];
        hasher.update(chunk);
        if chunk.iter().all(|&b| b == 0) {
            // Leave a hole: the pinned hash covers these zeros.
            out.seek(SeekFrom::Current(n as i64)).map_err(io_err)?;
        } else {
            out.write_all(chunk).map_err(io_err)?;
        }
        written += n as u64;
        progress(InstallStage::Unpacking, written, total);
    }
    if written != total {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "image unpacked to {written} bytes, pinned {total}"
        )));
    }
    out.set_len(total).map_err(io_err)?;
    out.sync_all().map_err(io_err)?;
    if hex::encode(hasher.finalize()) != image.disk.sha512 {
        return Err(ComputerError::ImageVerificationFailed(format!(
            "unpacked image {} does not match its pinned SHA-512",
            image.id
        )));
    }
    Ok(())
}

/// Remove leftovers of an abandoned install (staging and trash).
pub fn discard_staging(images_dir: &Path, id: &str) {
    let _ = fs::remove_dir_all(images_dir.join(".staging").join(id));
    let _ = fs::remove_dir_all(images_dir.join(".trash"));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A 1 MiB disk with a data prefix, a zero hole and a data tail.
    fn fake_disk() -> Vec<u8> {
        let mut disk = vec![0u8; 3 * CHUNK];
        for (i, b) in disk.iter_mut().take(4096).enumerate() {
            *b = (i % 251) as u8 + 1;
        }
        let n = disk.len();
        disk[n - 100..].fill(7);
        disk
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn entry(disk: &[u8], archive: &[u8]) -> ReleaseImage {
        let mut raw: serde_json::Value = serde_json::from_str(CATALOG_JSON).unwrap();
        let mut image: ReleaseImage =
            serde_json::from_value(raw["images"].as_array_mut().unwrap().remove(0)).unwrap();
        image.id = "pegoles-test-image".into();
        image.disk.bytes = disk.len() as u64;
        image.disk.sha512 = hex::encode(Sha512::digest(disk));
        image.archive.bytes = archive.len() as u64;
        image.archive.sha256 = hex::encode(Sha256::digest(archive));
        image.archive.urls = vec![
            "https://mirror-a.test/img.gz".into(),
            "https://mirror-b.test/img.gz".into(),
        ];
        image
    }

    /// Serves `bytes`; honours ranges; can cut the stream after `cut` bytes
    /// on the first `cuts` requests, or fail mirror A entirely.
    struct FakeSource {
        bytes: Vec<u8>,
        cut: usize,
        cuts: AtomicUsize,
        honour_range: bool,
        fail_first_mirror: bool,
        offline: std::sync::atomic::AtomicBool,
        requests: Mutex<Vec<(String, u64)>>,
    }

    impl FakeSource {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                cut: usize::MAX,
                cuts: AtomicUsize::new(0),
                honour_range: true,
                fail_first_mirror: false,
                offline: std::sync::atomic::AtomicBool::new(false),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    struct CutReader<R: Read> {
        inner: R,
        left: usize,
    }

    impl<R: Read> Read for CutReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "cut",
                ));
            }
            let max = buf.len().min(self.left);
            let n = self.inner.read(&mut buf[..max])?;
            self.left -= n;
            Ok(n)
        }
    }

    impl ArchiveSource for FakeSource {
        fn open(&self, url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool)> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_string(), offset));
            if self.offline.load(Ordering::SeqCst) {
                return Err(ComputerError::Backend("network down".into()));
            }
            if self.fail_first_mirror && url.contains("mirror-a") {
                return Err(ComputerError::Backend("mirror down".into()));
            }
            let (start, ranged) = if self.honour_range {
                (offset as usize, offset > 0)
            } else {
                (0, false)
            };
            let body = Cursor::new(self.bytes[start.min(self.bytes.len())..].to_vec());
            if self.cuts.load(Ordering::SeqCst) > 0 {
                self.cuts.fetch_sub(1, Ordering::SeqCst);
                return Ok((
                    Box::new(CutReader {
                        inner: body,
                        left: self.cut,
                    }),
                    ranged,
                ));
            }
            Ok((Box::new(body), ranged))
        }
    }

    fn run(dir: &Path, image: &ReleaseImage, src: &FakeSource) -> Result<PathBuf> {
        install(dir, image, src, &mut |_, _, _| {}, &|| false)
    }

    /// The Windows image is not published yet: the host sees no release
    /// entry for it, so setup reports it unavailable instead of installing
    /// the arm64 image.
    #[test]
    fn the_x64_image_is_not_offered_until_published() {
        assert!(release_image(crate::image::PEGOLES_BASE_IMAGE_ID_X64).is_none());
        assert!(catalog().unwrap().iter().all(|i| i.architecture == "arm64"));
    }

    #[cfg(not(windows))]
    #[test]
    fn builtin_catalog_is_valid_and_pins_the_product_image() {
        let images = catalog().unwrap();
        let product =
            release_image(crate::image::PEGOLES_PRODUCT_IMAGE_ID).expect("product image pinned");
        assert!(images.iter().all(|i| i.validate().is_ok()));
        assert_eq!(product.disk.file_name, "disk.raw");
        assert_eq!(
            product.guest_protocol_version,
            pegoles_guest_proto::GUEST_PROTOCOL_VERSION
        );
        assert!(product
            .provenance
            .sanitized
            .iter()
            .any(|s| s.contains("ssh")));
    }

    #[test]
    fn installs_exact_bytes_sparse_readonly_with_pinned_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        // Large enough that APFS makes the zero run a hole (it fills small
        // gaps of a few MiB); data at both ends like a real disk.
        let mut disk = vec![0u8; 72 * CHUNK];
        disk[..fake_disk().len()].copy_from_slice(&fake_disk());
        let n = disk.len();
        disk[n - 4096..].fill(9);
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let dest = run(tmp.path(), &image, &FakeSource::new(archive)).unwrap();
        let installed = dest.join("disk.raw");
        assert_eq!(fs::read(&installed).unwrap(), disk);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
            0o444
        );
        assert!(installed_matches_pin(&dest, &image));
        verify_installed(&dest, &image).unwrap();
        let manifest: DerivedManifest =
            serde_json::from_str(&fs::read_to_string(dest.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest, image.manifest());
        assert!(!tmp.path().join(".staging").join(&image.id).exists());
        // The zero run in the middle is a hole, not written blocks.
        use std::os::unix::fs::MetadataExt;
        assert!(fs::metadata(&installed).unwrap().blocks() * 512 < disk.len() as u64);
    }

    #[test]
    fn interrupted_download_resumes_from_the_staged_part() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut src = FakeSource::new(archive.clone());
        src.cut = archive.len() / 2;
        src.cuts = AtomicUsize::new(1);
        // A stream cut mid-way is resumed with a ranged request right away.
        run(tmp.path(), &image, &src).unwrap();
        let reqs = src.requests.lock().unwrap();
        assert!(
            reqs.iter()
                .any(|(_, off)| *off == (archive.len() / 2) as u64),
            "{reqs:?}"
        );
    }

    #[test]
    fn a_host_that_stops_adding_bytes_is_given_up_and_the_part_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut src = FakeSource::new(archive.clone());
        src.cut = archive.len() / 3;
        src.cuts = AtomicUsize::new(1);
        // Cut once, then every mirror refuses: bounded attempts, then an error.
        let first = install(
            tmp.path(),
            &image,
            &src,
            &mut |_, done, _| {
                if done > 0 {
                    src.offline.store(true, Ordering::SeqCst);
                }
            },
            &|| false,
        );
        assert!(first.is_err());
        let per_mirror = src.requests.lock().unwrap().len();
        assert!(
            per_mirror <= 1 + 2 * MAX_STALLED_ATTEMPTS as usize,
            "{per_mirror} requests"
        );
        let part = tmp
            .path()
            .join(".staging")
            .join(&image.id)
            .join(format!("{}.part", image.archive.file_name));
        let partial = fs::metadata(&part).unwrap().len();
        assert!(partial > 0 && partial < archive.len() as u64);
        // Back online: the next attempt resumes from the staged bytes.
        src.offline.store(false, Ordering::SeqCst);
        run(tmp.path(), &image, &src).unwrap();
        assert!(src
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(_, off)| *off == partial));
    }

    #[test]
    fn server_ignoring_range_restarts_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut src = FakeSource::new(archive.clone());
        src.cut = archive.len() / 3;
        src.cuts = AtomicUsize::new(1);
        src.honour_range = false;
        // First mirror cut, second mirror serves the whole file from 0.
        run(tmp.path(), &image, &src).unwrap();
    }

    #[test]
    fn falls_back_to_the_next_mirror() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut src = FakeSource::new(archive);
        src.fail_first_mirror = true;
        run(tmp.path(), &image, &src).unwrap();
    }

    #[test]
    fn tampered_archive_is_rejected_and_nothing_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut evil = archive.clone();
        let mid = evil.len() / 2;
        evil[mid] ^= 0xff;
        let err = run(tmp.path(), &image, &FakeSource::new(evil)).unwrap_err();
        assert!(
            matches!(err, ComputerError::ImageVerificationFailed(_)),
            "{err}"
        );
        assert!(!tmp.path().join(&image.id).exists());
        // The bad archive is not kept for a later "resume".
        assert!(!tmp
            .path()
            .join(".staging")
            .join(&image.id)
            .join(&image.archive.file_name)
            .exists());
    }

    #[test]
    fn oversized_archive_stream_is_cut_at_the_pin() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let mut bigger = archive.clone();
        bigger.extend_from_slice(&[0u8; 4096]);
        let err = run(tmp.path(), &image, &FakeSource::new(bigger)).unwrap_err();
        assert!(
            matches!(err, ComputerError::ImageVerificationFailed(_)),
            "{err}"
        );
        assert!(!tmp.path().join(&image.id).exists());
    }

    #[test]
    fn archive_that_unpacks_to_other_bytes_is_rejected() {
        // Correct archive pin, but the disk pin names different content:
        // e.g. a catalog/archive mismatch or a decompression bomb.
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let mut other = disk.clone();
        other.extend_from_slice(&[1u8; 512]);
        let archive = gz(&other);
        let image = entry(&disk, &archive);
        let err = run(tmp.path(), &image, &FakeSource::new(archive)).unwrap_err();
        assert!(
            matches!(err, ComputerError::ImageVerificationFailed(_)),
            "{err}"
        );
        assert!(!tmp.path().join(&image.id).exists());
        assert!(!tmp
            .path()
            .join(".staging")
            .join(&image.id)
            .join("disk.raw.part")
            .exists());
    }

    #[test]
    fn corrupt_gzip_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let garbage = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 0xde, 0xad, 0xbe, 0xef];
        let image = entry(&disk, &garbage);
        let err = run(tmp.path(), &image, &FakeSource::new(garbage)).unwrap_err();
        assert!(
            matches!(err, ComputerError::ImageVerificationFailed(_)),
            "{err}"
        );
    }

    #[test]
    fn cancel_stops_without_installing() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let err = install(
            tmp.path(),
            &image,
            &FakeSource::new(archive),
            &mut |_, _, _| {},
            &|| true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cancelled"));
        assert!(!tmp.path().join(&image.id).exists());
    }

    #[test]
    fn replaces_an_existing_invalid_copy_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let old = tmp.path().join(&image.id);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("disk.raw"), b"stale").unwrap();
        run(tmp.path(), &image, &FakeSource::new(archive)).unwrap();
        assert_eq!(fs::read(old.join("disk.raw")).unwrap(), disk);
    }

    #[test]
    fn tampered_install_fails_verification_and_status() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let image = entry(&disk, &archive);
        let dest = run(tmp.path(), &image, &FakeSource::new(archive)).unwrap();
        let path = dest.join("disk.raw");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[10] ^= 1;
        fs::write(&path, &bytes).unwrap();
        // Same size, same manifest: only hashing catches it.
        assert!(installed_matches_pin(&dest, &image));
        assert!(matches!(
            verify_installed(&dest, &image),
            Err(ComputerError::ImageVerificationFailed(_))
        ));
        // A symlinked disk is refused outright.
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", &path).unwrap();
        assert!(!installed_matches_pin(&dest, &image));
        assert!(verify_installed(&dest, &image).is_err());
    }

    #[test]
    fn catalog_validation_rejects_unsafe_entries() {
        let disk = fake_disk();
        let archive = gz(&disk);
        let good = entry(&disk, &archive);
        let mut bad = good.clone();
        bad.archive.urls = vec!["http://insecure.test/img.gz".into()];
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.id = "../escape".into();
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.disk.sha512 = "ab".into();
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.archive.compression = "xz".into();
        assert!(bad.validate().is_err());
        assert!(parse_catalog(r#"{"schema":1,"images":[],"extra":1}"#).is_err());
        assert!(parse_catalog(r#"{"schema":2,"images":[]}"#).is_err());
    }

    #[test]
    fn unpublished_catalog_refuses_to_download() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = fake_disk();
        let archive = gz(&disk);
        let mut image = entry(&disk, &archive);
        image.archive.urls = vec![format!(
            "https://example.invalid/{UNPUBLISHED_MARKER}/img.gz"
        )];
        assert!(!image.is_published());
        let src = FakeSource::new(archive);
        assert!(matches!(
            run(tmp.path(), &image, &src),
            Err(ComputerError::ImageMissing(_))
        ));
        assert!(src.requests.lock().unwrap().is_empty());
    }
}

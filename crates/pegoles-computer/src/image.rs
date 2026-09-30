//! Base image management: download once from official Debian, verify,
//! extract read-only, then copy per computer.
//!
//! SECURITY INVARIANTS:
//! - Downloads happen ONLY from `DEBIAN_BASE_URL` (official Debian, TLS).
//! - The expected SHA-512 comes from the `SHA512SUMS` file fetched from the
//!   same official directory over TLS. No checksum is ever invented.
//! - Verification failure deletes the artifact and returns
//!   `ImageVerificationFailed`. Fail closed: unverified images never boot.
//! - The base raw is stored read-only; each computer gets its own writable
//!   copy. The base is never modified.
//! - Spawning `/usr/bin/tar` below is first-party installer behavior
//!   (decompressing a verified archive), NOT model-generated host execution.
//!   The model can never reach this code path with arbitrary arguments.

use pegoles_protocol::ComputerId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{ComputerError, Result};
use crate::platform::{DiskFormat, GuestArchitecture, HostPlatform};

/// Official Debian cloud image directory (TLS). The ONLY download source.
pub const DEBIAN_BASE_URL: &str = "https://cdimage.debian.org/images/cloud/trixie/latest/";
/// Compressed raw disk image (extracts to the `.raw` consumed by Vz).
pub const DEBIAN_ARTIFACT: &str = "debian-13-nocloud-arm64.tar.xz";
/// Checksum list published by Debian in the same directory.
pub const DEBIAN_CHECKSUM_FILE: &str = "SHA512SUMS";

/// An official Debian image spec (download source for image builders).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSpec {
    pub id: &'static str,
    pub distribution: &'static str,
    pub version: &'static str,
    pub architecture: &'static str,
    pub format: &'static str,
    pub base_url: &'static str,
    pub artifact: &'static str,
    pub checksum_file: &'static str,
}

pub const PEGOLES_DEBIAN_13_ARM64: ImageSpec = ImageSpec {
    id: "pegoles-debian-13-arm64",
    distribution: "debian",
    version: "13",
    architecture: "arm64",
    format: "raw",
    base_url: DEBIAN_BASE_URL,
    artifact: DEBIAN_ARTIFACT,
    checksum_file: DEBIAN_CHECKSUM_FILE,
};

/// Build-time source WITH cloud-init (used once by the image builder to
/// install the guest runtime via a NoCloud seed ISO; never booted in the
/// normal lifecycle). Same official directory, same verification.
pub const GENERIC_DEBIAN_13_ARM64: ImageSpec = ImageSpec {
    id: "debian-13-generic-arm64",
    distribution: "debian",
    version: "13",
    architecture: "arm64",
    format: "raw",
    base_url: DEBIAN_BASE_URL,
    artifact: "debian-13-generic-arm64.tar.xz",
    checksum_file: DEBIAN_CHECKSUM_FILE,
};

/// Windows build-time source: official Debian 13 generic amd64
/// (cloud-init included for first-boot provisioning on Windows hardware).
/// Converted to VHDX by build tooling only; never booted as RAW.
pub const GENERIC_DEBIAN_13_AMD64: ImageSpec = ImageSpec {
    id: "debian-13-generic-amd64",
    distribution: "debian",
    version: "13",
    architecture: "amd64",
    format: "raw",
    base_url: DEBIAN_BASE_URL,
    artifact: "debian-13-generic-amd64.tar.xz",
    checksum_file: DEBIAN_CHECKSUM_FILE,
};

/// Pegoles derived image (built once from the official source).
pub const PEGOLES_BASE_IMAGE_ID: &str = "pegoles-base-0.1";
pub const PEGOLES_IMAGE_VERSION: &str = "0.1";
/// Phase 5.1: second derived image (graphical + input/capture). Sealed
/// beside v0.1, never over it.
pub const PEGOLES_BASE_IMAGE_ID_V2: &str = "pegoles-base-0.2";
pub const PEGOLES_IMAGE_VERSION_V2: &str = "0.2";
/// Third derived image: v0.2 + authenticated guest runtime (reserved
/// vsock source port), correct frame channel order, workspace terminal
/// instead of the dev fixture, network/remote-login services disabled.
pub const PEGOLES_BASE_IMAGE_ID_V3: &str = "pegoles-base-0.3";
pub const PEGOLES_IMAGE_VERSION_V3: &str = "0.3";
/// The x64 image for Windows (Hyper-V): Debian 13 amd64 provisioned by
/// `scripts/build-guest-image/build-x64.sh`, a VHDX, runtime in listen
/// mode. Published as the immutable GitHub release `guest-image-x64-0.1`
/// and pinned in `catalog/images.json` (disk `disk.vhdx`).
pub const PEGOLES_BASE_IMAGE_ID_X64: &str = "pegoles-base-x64-0.1";
/// The image a normal create boots on this host: the arm64 image on
/// macOS, the x64 one on Windows. Older arm64 images predate the runtime
/// authentication the host now requires and cannot connect.
pub const PEGOLES_PRODUCT_IMAGE_ID: &str = if cfg!(windows) {
    PEGOLES_BASE_IMAGE_ID_X64
} else {
    PEGOLES_BASE_IMAGE_ID_V3
};
/// Env override selecting the boot image (dev/provisioning). Unset →
/// the product image. Unknown values fail closed (missing dir →
/// `Missing`, never a silent fallback to another image).
pub const IMAGE_ID_ENV: &str = "PEGOLES_IMAGE_ID";

/// Which derived image this process boots: the product image unless
/// overridden. Unknown values are returned as-is: their directory does
/// not exist, so status resolves to `Missing` with an explicit error
/// (fail closed, never a silent fallback to another image).
pub fn active_image_id() -> String {
    // Release builds always boot the product image.
    if !cfg!(debug_assertions) {
        return PEGOLES_PRODUCT_IMAGE_ID.to_string();
    }
    // Debug override: a plain directory name only (never a path).
    match std::env::var(IMAGE_ID_ENV).map(|v| v.trim().to_string()) {
        Ok(v) if !v.is_empty() && !v.contains(['/', '\\']) && !v.starts_with('.') => v,
        _ => PEGOLES_PRODUCT_IMAGE_ID.to_string(),
    }
}

/// Logical image: one identity across platform artifacts.
/// `pegoles-debian-13-arm64.raw` is NOT "the Pegoles image" — it is the
/// macOS/arm64 artifact of "Pegoles Base Image v0.1". Windows/amd64 will
/// be `pegoles-debian-13-amd64.vhdx` of the same logical image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageFamily {
    pub image_id: String,
    pub pegoles_image_version: String,
    pub debian_version: String,
    pub guest_runtime_version: String,
    pub guest_protocol_version: u32,
}

impl ImageFamily {
    pub fn v0_1() -> Self {
        Self {
            image_id: PEGOLES_BASE_IMAGE_ID.to_string(),
            pegoles_image_version: PEGOLES_IMAGE_VERSION.to_string(),
            debian_version: "13".to_string(),
            guest_runtime_version: pegoles_guest_proto::RUNTIME_VERSION.to_string(),
            guest_protocol_version: pegoles_guest_proto::GUEST_PROTOCOL_VERSION,
        }
    }
}

/// One platform artifact of a logical image: (host, guest arch, format)
/// maps to exactly one file with its own checksum. Hashes are never
/// shared across artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformArtifact {
    pub host: HostPlatform,
    pub guest_arch: GuestArchitecture,
    pub format: DiskFormat,
    pub file_name: String,
}

/// The artifact matrix: present (verified here) vs planned.
/// macOS/arm64 RAW is booted and verified; Windows/amd64 VHDX bytes are
/// built, hashed, and kernel-gated but never booted (no Windows hardware
/// in this phase); everything else is future work, not silent gaps.
pub fn known_artifacts() -> Vec<(PlatformArtifact, ArtifactStatus)> {
    vec![
        (
            PlatformArtifact {
                host: HostPlatform::MacOS,
                guest_arch: GuestArchitecture::Arm64,
                format: DiskFormat::Raw,
                file_name: "pegoles-debian-13-arm64.raw".to_string(),
            },
            ArtifactStatus::Available,
        ),
        (
            PlatformArtifact {
                host: HostPlatform::Windows,
                guest_arch: GuestArchitecture::X86_64,
                format: DiskFormat::Vhdx,
                file_name: "disk.vhdx".to_string(),
            },
            ArtifactStatus::Built,
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    /// Booted and verified end-to-end.
    Available,
    /// Bytes built, hashed, and kernel-gated — never booted on hardware.
    Built,
    /// Specified only.
    Planned,
}

/// Resolve the disk format for a (host, guest arch) pair. Used by the
/// builder to pick artifact handling; unknown pairs are an explicit
/// error, never a default guess.
pub fn disk_format_for(host: HostPlatform, arch: GuestArchitecture) -> Option<DiskFormat> {
    known_artifacts()
        .into_iter()
        .find(|(a, _)| a.host == host && a.guest_arch == arch)
        .map(|(a, _)| a.format)
}

/// One built artifact record inside the manifest: name + format + the
/// hash OF THOSE BYTES, plus the per-artifact build facts that differ
/// across platforms (arch, runtime, source hash, build time). Top-level
/// manifest scalars mirror the PRIMARY (first-built) artifact for backward
/// compatibility; `artifacts[]` is authoritative per platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub file_name: String,
    pub disk_format: DiskFormat,
    pub sha512: String,
    pub bytes: u64,
    pub architecture: String,
    pub guest_runtime_version: String,
    pub source_image_sha512: String,
    pub built_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageStatus {
    /// No usable base image on disk.
    Missing,
    /// A partial download exists (preparation can resume/restart it).
    Downloading,
    /// Verified base raw present and read-only.
    Ready,
    /// Raw present but no verification marker: never boot this.
    Invalid,
}

/// Preparation stage reported to progress callbacks (real bytes, no timers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareStage {
    FetchingChecksums,
    Downloading,
    Verifying,
    Extracting,
}

impl PrepareStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrepareStage::FetchingChecksums => "fetching_checksums",
            PrepareStage::Downloading => "downloading",
            PrepareStage::Verifying => "verifying",
            PrepareStage::Extracting => "extracting",
        }
    }
}

/// A verified base image on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerImage {
    pub spec: ImageSpec,
    pub base_raw: PathBuf,
    pub sha512: String,
}

pub struct ComputerImageManager {
    images_dir: PathBuf,
    spec: ImageSpec,
}

impl ComputerImageManager {
    pub fn new(images_dir: PathBuf) -> Self {
        Self {
            images_dir,
            spec: PEGOLES_DEBIAN_13_ARM64,
        }
    }

    /// Manager for a different official spec (e.g. the build-time source
    /// with cloud-init). Same verification rules apply.
    pub fn with_spec(images_dir: PathBuf, spec: ImageSpec) -> Self {
        Self { images_dir, spec }
    }

    pub fn spec(&self) -> ImageSpec {
        self.spec
    }

    fn spec_paths(&self, spec: &ImageSpec) -> ImagePaths {
        let dir = self.images_dir.join(spec.id);
        ImagePaths {
            dir: dir.clone(),
            archive: dir.join(spec.artifact),
            partial: dir.join(format!("{}.part", spec.artifact)),
            base_raw: dir.join("base.raw"),
            marker: dir.join("base.raw.verified"),
        }
    }

    pub fn base_raw_path(&self) -> PathBuf {
        self.spec_paths(&self.spec).base_raw
    }

    pub fn status(&self) -> ImageStatus {
        self.status_for(&self.spec)
    }

    fn status_for(&self, spec: &ImageSpec) -> ImageStatus {
        let p = self.spec_paths(spec);
        if p.marker.exists() && p.base_raw.exists() {
            ImageStatus::Ready
        } else if p.partial.exists() {
            ImageStatus::Downloading
        } else if p.base_raw.exists() {
            ImageStatus::Invalid
        } else {
            ImageStatus::Missing
        }
    }

    /// Load the verified image handle. Fails closed on anything but Ready.
    pub fn load(&self) -> Result<ComputerImage> {
        let spec = self.spec;
        match self.status_for(&spec) {
            ImageStatus::Ready => {
                let p = self.spec_paths(&spec);
                let sha512 = fs::read_to_string(&p.marker)
                    .map_err(|e| ComputerError::Backend(e.to_string()))?;
                Ok(ComputerImage {
                    spec,
                    base_raw: p.base_raw,
                    sha512: sha512.trim().to_string(),
                })
            }
            ImageStatus::Missing | ImageStatus::Downloading => {
                Err(ComputerError::ImageMissing(spec.id.to_string()))
            }
            ImageStatus::Invalid => Err(ComputerError::ImageVerificationFailed(format!(
                "{} present without verification marker; refusing to boot",
                spec.id
            ))),
        }
    }

    /// Copy the verified base into a per-computer writable disk.
    /// The base stays read-only and untouched. Copies inherit the base's
    /// read-only mode, so the destination is explicitly made writable:
    /// Virtualization.framework rejects read-only files opened read-write.
    pub fn instantiate(&self, computer_id: &ComputerId, dest_disk: &Path) -> Result<()> {
        let image = self.load()?;
        instantiate_from(&image.base_raw, dest_disk)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let base_ro = fs::Permissions::from_mode(0o444);
            let _ = fs::set_permissions(&image.base_raw, base_ro);
        }
        let _ = computer_id;
        Ok(())
    }

    /// Full preparation: checksums -> download -> verify -> extract -> Ready.
    /// `progress(stage, downloaded_bytes, total_bytes)` receives real counts.
    pub fn prepare(
        &self,
        progress: &mut dyn FnMut(PrepareStage, u64, u64),
    ) -> Result<ComputerImage> {
        let spec = self.spec;
        let p = self.spec_paths(&spec);
        fs::create_dir_all(&p.dir).map_err(|e| ComputerError::Backend(e.to_string()))?;

        if self.status_for(&spec) == ImageStatus::Ready {
            return self.load();
        }

        // 1. Official checksums over TLS from the same directory.
        progress(PrepareStage::FetchingChecksums, 0, 1);
        let sums_url = format!("{}{}", spec.base_url, spec.checksum_file);
        let sums = http_get_text(&sums_url)?;
        let expected = parse_sha512sums(&sums, spec.artifact).ok_or_else(|| {
            ComputerError::ImageVerificationFailed(format!(
                "checksum entry for {} not found in official {}",
                spec.artifact, spec.checksum_file
            ))
        })?;

        // 2. Download (resume into .part).
        let artifact_url = format!("{}{}", spec.base_url, spec.artifact);
        download_to(&artifact_url, &p.partial, &mut |d, t| {
            progress(PrepareStage::Downloading, d, t)
        })?;
        fs::rename(&p.partial, &p.archive).map_err(|e| ComputerError::Backend(e.to_string()))?;

        // 3. Verify before anything else touches the bytes.
        progress(PrepareStage::Verifying, 0, 1);
        let actual = sha512_file(&p.archive)?;
        if actual != expected.to_lowercase() {
            let _ = fs::remove_file(&p.archive);
            return Err(ComputerError::ImageVerificationFailed(format!(
                "checksum mismatch for {}",
                spec.artifact
            )));
        }

        // 4. Extract verified archive with the system tar (first-party step).
        progress(PrepareStage::Extracting, 0, 1);
        extract_tar_xz(&p.archive, &p.dir)?;
        let extracted_raw = find_extracted_raw(&p.dir).ok_or_else(|| {
            ComputerError::ImageVerificationFailed("no .raw inside verified archive".to_string())
        })?;
        if extracted_raw != p.base_raw {
            fs::rename(&extracted_raw, &p.base_raw)
                .map_err(|e| ComputerError::Backend(e.to_string()))?;
        }
        fs::write(&p.marker, &actual).map_err(|e| ComputerError::Backend(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = fs::Permissions::from_mode(0o444);
            let _ = fs::set_permissions(&p.base_raw, ro);
        }
        let _ = fs::remove_file(&p.archive);
        progress(PrepareStage::Extracting, 1, 1);

        self.load()
    }

    // --- Pegoles derived base image (built once, booted always) ---

    fn derived_dir(&self) -> PathBuf {
        self.derived_dir_for(&active_image_id())
    }

    fn derived_dir_for(&self, image_id: &str) -> PathBuf {
        self.images_dir.join(image_id)
    }

    fn derived_paths(&self) -> DerivedPaths {
        self.derived_paths_for(DiskFormat::Raw)
    }

    fn derived_paths_for(&self, format: DiskFormat) -> DerivedPaths {
        self.derived_paths_for_image(&active_image_id(), format)
    }

    fn derived_paths_for_image(&self, image_id: &str, format: DiskFormat) -> DerivedPaths {
        let dir = self.derived_dir_for(image_id);
        let disk_name = format!("disk.{}", format.extension());
        DerivedPaths {
            marker: dir.join(format!("{disk_name}.verified")),
            manifest: dir.join("manifest.json"),
            disk: dir.join(disk_name),
        }
    }

    /// Status driven by the manifest: the primary artifact file it names
    /// must exist alongside marker + manifest. Format-agnostic, so RAW
    /// and VHDX derived images share one code path with separate hashes.
    pub fn derived_status(&self) -> ImageStatus {
        self.derived_status_for(&active_image_id())
    }

    /// Status of one named derived image (v0.1 default; v0.2 for Phase 5.1).
    /// Release builds additionally require the installed image to match
    /// the pin compiled into the app (`catalog/images.json`); anything else
    /// is `Invalid` and never boots.
    pub fn derived_status_for(&self, image_id: &str) -> ImageStatus {
        self.derived_status_with(image_id, crate::image_release::pins_enforced())
    }

    fn derived_status_with(&self, image_id: &str, enforce_pins: bool) -> ImageStatus {
        let status = self.derived_status_unpinned(image_id);
        if !enforce_pins || status != ImageStatus::Ready {
            return status;
        }
        match crate::image_release::release_image(image_id) {
            Some(pin)
                if crate::image_release::installed_matches_pin(
                    &self.derived_dir_for(image_id),
                    pin,
                ) =>
            {
                ImageStatus::Ready
            }
            _ => ImageStatus::Invalid,
        }
    }

    fn derived_status_unpinned(&self, image_id: &str) -> ImageStatus {
        let dir = self.derived_dir_for(image_id);
        let manifest_path = dir.join("manifest.json");
        let manifest: Option<DerivedManifest> = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok());
        match manifest.and_then(|m| m.artifacts.first().cloned()) {
            Some(record) => {
                let disk = dir.join(&record.file_name);
                let marker = dir.join(format!("{}.verified", record.file_name));
                if disk.exists() && marker.exists() {
                    ImageStatus::Ready
                } else if disk.exists() {
                    ImageStatus::Invalid
                } else {
                    ImageStatus::Missing
                }
            }
            None => {
                // Legacy layout (Phase 3, disk.raw only) or absent.
                let p = self.derived_paths();
                if p.marker.exists() && p.disk.exists() {
                    ImageStatus::Ready
                } else if p.disk.exists() {
                    ImageStatus::Invalid
                } else {
                    ImageStatus::Missing
                }
            }
        }
    }

    pub fn derived_manifest(&self) -> Result<DerivedManifest> {
        self.derived_manifest_for(&active_image_id())
    }

    /// Manifest of one named derived image.
    pub fn derived_manifest_for(&self, image_id: &str) -> Result<DerivedManifest> {
        if self.derived_status_for(image_id) != ImageStatus::Ready {
            return Err(ComputerError::ImageMissing(image_id.to_string()));
        }
        let raw = fs::read_to_string(
            &self
                .derived_paths_for_image(image_id, DiskFormat::Raw)
                .manifest,
        )
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
        serde_json::from_str(&raw).map_err(|e| ComputerError::Backend(e.to_string()))
    }

    /// Hash the installed image against its release pin (once per app
    /// session per file). Fails closed when the image has no pin.
    pub fn verify_pinned(&self, image_id: &str) -> Result<()> {
        let pin = crate::image_release::release_image(image_id).ok_or_else(|| {
            ComputerError::ImageVerificationFailed(format!(
                "{image_id} is not a released Pegoles image"
            ))
        })?;
        crate::image_release::verify_installed(&self.derived_dir_for(image_id), pin)
    }

    /// Download, verify and install the pinned release image this build
    /// boots (`active_image_id`). See `image_release::install`.
    pub fn install_release_image(
        &self,
        progress: &mut dyn FnMut(crate::image_release::InstallStage, u64, u64),
        cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf> {
        let id = active_image_id();
        let pin = crate::image_release::release_image(&id).ok_or_else(|| {
            ComputerError::ImageMissing(format!("{id} has no release download in this build"))
        })?;
        if let Some(local) = crate::image_release::dev_archive_override() {
            // Debug builds: a local copy of the archive stands in for the
            // (possibly unpublished) URL; the digests are the same pins.
            let mut local_pin = pin.clone();
            local_pin.archive.urls =
                vec![format!("https://local.archive/{}", pin.archive.file_name)];
            return crate::image_release::install(
                &self.images_dir,
                &local_pin,
                &crate::image_release::LocalArchiveSource(local),
                progress,
                cancel,
            );
        }
        crate::image_release::install(
            &self.images_dir,
            pin,
            &crate::image_release::HttpsSource::default(),
            progress,
            cancel,
        )
    }

    /// Seal a provisioned work disk as the derived base image: copy it in,
    /// hash the bytes, write marker + manifest. Nothing is invented: both
    /// checksums are computed from real bytes on disk.
    pub fn publish_derived(
        &self,
        work_disk: &Path,
        manifest: DerivedManifestInput,
    ) -> Result<DerivedManifest> {
        self.publish_derived_as(work_disk, manifest, "disk.raw", DiskFormat::Raw)
    }

    /// Same, with explicit artifact file name + format (e.g. `disk.vhdx`
    /// for the Windows artifact — independently hashed, never shared).
    /// Seals into `manifest.image_id` (v0.1 default path preserved).
    pub fn publish_derived_as(
        &self,
        work_disk: &Path,
        manifest: DerivedManifestInput,
        file_name: &str,
        format: DiskFormat,
    ) -> Result<DerivedManifest> {
        let dir = self.images_dir.join(&manifest.image_id);
        let disk = dir.join(file_name);
        let marker = dir.join(format!("{file_name}.verified"));
        fs::create_dir_all(&dir).map_err(|e| ComputerError::Backend(e.to_string()))?;
        // Re-publish must replace a previous read-only artifact: make the
        // destination writable first (idempotent builder runs).
        #[cfg(unix)]
        if disk.exists() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&disk, fs::Permissions::from_mode(0o644));
        }
        fs::copy(work_disk, &disk).map_err(|e| ComputerError::Backend(e.to_string()))?;
        let image_sha512 = sha512_file(&disk)?;
        let bytes = fs::metadata(&disk)
            .map_err(|e| ComputerError::Backend(e.to_string()))?
            .len();
        let built_at = chrono::Utc::now().to_rfc3339();
        let record = ArtifactRecord {
            file_name: file_name.to_string(),
            disk_format: format,
            sha512: image_sha512.clone(),
            bytes,
            architecture: manifest.architecture.clone(),
            guest_runtime_version: manifest.guest_runtime_version.clone(),
            source_image_sha512: manifest.source_image_sha512.clone(),
            built_at: built_at.clone(),
        };
        let manifest_path = dir.join("manifest.json");
        // Merge into any existing manifest (multi-artifact logical image).
        // Top-level scalars stay pinned to the PRIMARY (first-built)
        // artifact — EXCEPT when resealing the same file name (rebuild of
        // the primary): then the fresh record's scalars win, or the
        // manifest would describe bytes that no longer exist (found by
        // Phase 5.1 reseal verification). The new record always carries
        // its own authoritative facts.
        let previous: Option<DerivedManifest> = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok());
        let mut artifacts: Vec<ArtifactRecord> = previous
            .as_ref()
            .map(|m| m.artifacts.clone())
            .unwrap_or_default();
        let replaces_primary = previous
            .as_ref()
            .and_then(|m| m.artifacts.first())
            .is_some_and(|a| a.file_name == record.file_name);
        artifacts.retain(|a| a.file_name != record.file_name);
        artifacts.push(record);
        let (top_arch, top_runtime, top_source, top_sha, top_built) = match &previous {
            Some(m) if !replaces_primary => (
                m.architecture.clone(),
                m.guest_runtime_version.clone(),
                m.source_image_sha512.clone(),
                m.image_sha512.clone(),
                m.built_at.clone(),
            ),
            _ => (
                manifest.architecture.clone(),
                manifest.guest_runtime_version.clone(),
                manifest.source_image_sha512.clone(),
                image_sha512.clone(),
                built_at,
            ),
        };
        let full = DerivedManifest {
            image_id: manifest.image_id.clone(),
            pegoles_image_version: manifest.image_version.clone(),
            debian_version: manifest.debian_version,
            architecture: top_arch,
            guest_runtime_version: top_runtime,
            guest_protocol_version: manifest.guest_protocol_version,
            source_image_sha512: top_source,
            image_sha512: top_sha,
            built_at: top_built,
            artifacts,
            // Graphical stack + capabilities: the builder's fresh facts
            // win when provided; otherwise keep whatever a previous build
            // recorded (multi-artifact merge never invents them).
            graphical: manifest
                .graphical
                .clone()
                .or_else(|| previous.as_ref().and_then(|m| m.graphical.clone())),
            capabilities: if manifest.capabilities.is_empty() {
                previous.map(|m| m.capabilities).unwrap_or_default()
            } else {
                manifest.capabilities.clone()
            },
        };
        let manifest_path = dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&full).expect("manifest serializes"),
        )
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
        fs::write(&marker, &image_sha512).map_err(|e| ComputerError::Backend(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = fs::Permissions::from_mode(0o444);
            let _ = fs::set_permissions(&disk, ro);
        }
        Ok(full)
    }

    /// Copy the derived image into a per-computer writable disk.
    pub fn instantiate_derived(&self, computer_id: &ComputerId, dest_disk: &Path) -> Result<()> {
        let src = self.derived_disk_path()?;
        instantiate_from(&src, dest_disk)?;
        let _ = computer_id;
        Ok(())
    }

    fn derived_disk_path(&self) -> Result<PathBuf> {
        self.derived_disk_for_format(DiskFormat::Raw)
    }

    /// Disk path for a specific format (Windows resolves VHDX, macOS RAW).
    /// Fails closed when that platform artifact was never built.
    pub fn derived_disk_for_format(&self, format: DiskFormat) -> Result<PathBuf> {
        if self.derived_status() != ImageStatus::Ready {
            return Err(ComputerError::ImageMissing(active_image_id()));
        }
        let manifest = self.derived_manifest()?;
        let record = manifest
            .artifacts
            .iter()
            .find(|a| a.disk_format == format)
            .ok_or_else(|| {
                ComputerError::ImageMissing(format!(
                    "{} has no {:?} artifact",
                    active_image_id(),
                    format
                ))
            })?;
        Ok(self.derived_dir().join(&record.file_name))
    }

    /// Boot source resolution: derived Pegoles image when Ready, else the
    /// official image for this manager's spec (when `allow_official`).
    /// Missing both fails closed. Windows passes false: a RAW fallback
    /// could never boot on Hyper-V.
    pub fn boot_source(&self) -> Result<BootSource> {
        self.boot_source_with_fallback(true)
    }

    pub fn boot_source_with_fallback(&self, allow_official: bool) -> Result<BootSource> {
        self.boot_source_for_format_with_fallback(DiskFormat::Raw, allow_official)
    }

    /// Format-aware resolution (Windows asks for Vhdx, macOS for Raw).
    /// Build-time exception (found by Phase 5.1 hardware provisioning):
    /// when `PEGOLES_SEED_ISO` is set, an image builder is provisioning
    /// FROM the official cloud image, so the derived preference is
    /// skipped — otherwise builders could never reprovision once ANY
    /// derived image is Ready (they would clone the old image, whose
    /// cloud-init already ran, and idle forever waiting for poweroff).
    /// Normal boots never set the seed env and are unaffected.
    pub fn boot_source_for_format_with_fallback(
        &self,
        format: DiskFormat,
        allow_official: bool,
    ) -> Result<BootSource> {
        // Builders exist only in debug builds; a release never skips the
        // sealed image because of an environment variable.
        let building = cfg!(debug_assertions)
            && std::env::var("PEGOLES_SEED_ISO")
                .ok()
                .is_some_and(|p| !p.trim().is_empty());
        if !building && self.derived_status() == ImageStatus::Ready {
            if let Ok(disk) = self.derived_disk_for_format(format) {
                return Ok(BootSource::Derived(disk));
            }
            // Derived exists but not for this format: fall through to the
            // (possibly disabled) official path, which fails closed below.
        }
        if !allow_official {
            return Err(ComputerError::ImageMissing(format!(
                "Pegoles image {} is not installed ({:?})",
                active_image_id(),
                format
            )));
        }
        Ok(BootSource::Official(self.load()?))
    }

    /// Copy whichever boot source resolves into a per-computer disk.
    /// This is what backend `create` uses (macOS allows the official
    /// fallback; Windows resolves derived-only — see engine profile).
    pub fn instantiate_boot_source(
        &self,
        computer_id: &ComputerId,
        dest_disk: &Path,
    ) -> Result<BootSource> {
        self.instantiate_boot_source_with_fallback(computer_id, dest_disk, true)
    }

    pub fn instantiate_boot_source_with_fallback(
        &self,
        computer_id: &ComputerId,
        dest_disk: &Path,
        allow_official: bool,
    ) -> Result<BootSource> {
        self.instantiate_boot_source_for_format_with_fallback(
            computer_id,
            dest_disk,
            DiskFormat::Raw,
            allow_official,
        )
    }

    pub fn instantiate_boot_source_for_format_with_fallback(
        &self,
        computer_id: &ComputerId,
        dest_disk: &Path,
        format: DiskFormat,
        allow_official: bool,
    ) -> Result<BootSource> {
        self.instantiate_boot_source_with(
            computer_id,
            dest_disk,
            format,
            allow_official,
            crate::image_release::pins_enforced(),
        )
    }

    fn instantiate_boot_source_with(
        &self,
        computer_id: &ComputerId,
        dest_disk: &Path,
        format: DiskFormat,
        allow_official: bool,
        enforce_pins: bool,
    ) -> Result<BootSource> {
        let source = self.boot_source_for_format_with_fallback(format, allow_official)?;
        match &source {
            BootSource::Derived(raw) => {
                if enforce_pins {
                    // Clone exactly the file that was just hashed against the
                    // pin, never a path taken from the manifest on disk.
                    let id = active_image_id();
                    self.verify_pinned(&id)?;
                    let pin = crate::image_release::release_image(&id).ok_or_else(|| {
                        ComputerError::ImageVerificationFailed(format!("{id} has no pin"))
                    })?;
                    instantiate_from(
                        &self.derived_dir_for(&id).join(&pin.disk.file_name),
                        dest_disk,
                    )?;
                } else {
                    instantiate_from(raw, dest_disk)?;
                }
            }
            BootSource::Official(_) => {
                self.instantiate(computer_id, dest_disk)?;
            }
        }
        let _ = computer_id;
        Ok(source)
    }

    /// Reconcile an already-sealed artifact file into the manifest without
    /// copying: hash the bytes in place, verify against the on-disk marker
    /// when present, merge the record. Used to repair/extend manifests and
    /// by builders that place bytes out of band. Never invents hashes.
    pub fn reconcile_artifact(
        &self,
        file_name: &str,
        format: DiskFormat,
        architecture: String,
        guest_runtime_version: String,
        source_image_sha512: String,
    ) -> Result<DerivedManifest> {
        let dir = self.derived_dir();
        let disk = dir.join(file_name);
        let marker = dir.join(format!("{file_name}.verified"));
        let computed = sha512_file(&disk)?;
        if let Ok(marked) = fs::read_to_string(&marker) {
            let marked = marked.trim().to_string();
            if marked != computed {
                return Err(ComputerError::ImageVerificationFailed(format!(
                    "{file_name}: marker mismatch, refusing"
                )));
            }
        } else {
            fs::write(&marker, &computed).map_err(|e| ComputerError::Backend(e.to_string()))?;
        }
        let bytes = fs::metadata(&disk)
            .map_err(|e| ComputerError::Backend(e.to_string()))?
            .len();
        let built_at = chrono::Utc::now().to_rfc3339();
        let record = ArtifactRecord {
            file_name: file_name.to_string(),
            disk_format: format,
            sha512: computed,
            bytes,
            architecture,
            guest_runtime_version,
            source_image_sha512,
            built_at,
        };
        let manifest_path = dir.join("manifest.json");
        let mut full: DerivedManifest = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(DerivedManifest {
                image_id: PEGOLES_BASE_IMAGE_ID.to_string(),
                pegoles_image_version: PEGOLES_IMAGE_VERSION.to_string(),
                debian_version: "13".to_string(),
                architecture: record.architecture.clone(),
                guest_runtime_version: record.guest_runtime_version.clone(),
                guest_protocol_version: pegoles_guest_proto::GUEST_PROTOCOL_VERSION,
                source_image_sha512: record.source_image_sha512.clone(),
                image_sha512: record.sha512.clone(),
                built_at: record.built_at.clone(),
                artifacts: Vec::new(),
                graphical: None,
                capabilities: Vec::new(),
            });
        full.artifacts.retain(|a| a.file_name != record.file_name);
        full.artifacts.push(record);
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&full).expect("manifest serializes"),
        )
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
        Ok(full)
    }
}

/// Where a new computer's disk is copied from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootSource {
    Derived(PathBuf),
    Official(ComputerImage),
}

/// Manifest inputs supplied by the builder (versions it knows); the
/// content hashes are always computed here, never passed in.
#[derive(Clone, Debug)]
pub struct DerivedManifestInput {
    /// Target derived image id (v0.1 default; v0.2 for Phase 5.1).
    pub image_id: String,
    /// Pegoles image version string for the manifest.
    pub image_version: String,
    pub debian_version: String,
    pub architecture: String,
    pub guest_runtime_version: String,
    pub guest_protocol_version: u32,
    pub source_image_sha512: String,
    /// Graphical stack the builder actually installed (`None` = headless).
    /// Recorded from real installs, never assumed.
    pub graphical: Option<GraphicalImageInfo>,
    /// Guest capabilities this image provides (e.g. `["input", "frame"]`).
    pub capabilities: Vec<String>,
}

impl DerivedManifestInput {
    /// Same input, sealed as the image this process boots (tests).
    #[cfg(test)]
    pub(crate) fn for_active_image(mut self) -> Self {
        self.image_id = active_image_id();
        self
    }

    /// v0.1-shaped input (headless, no caps): preserves legacy call sites.
    pub fn v0_1(
        debian_version: String,
        architecture: String,
        guest_runtime_version: String,
        guest_protocol_version: u32,
        source_image_sha512: String,
    ) -> Self {
        Self {
            image_id: PEGOLES_BASE_IMAGE_ID.to_string(),
            image_version: PEGOLES_IMAGE_VERSION.to_string(),
            debian_version,
            architecture,
            guest_runtime_version,
            guest_protocol_version,
            source_image_sha512,
            graphical: None,
            capabilities: Vec::new(),
        }
    }
}

/// On-disk manifest of the derived image. `image_sha512` (derived bytes)
/// and `source_image_sha512` (official bytes) are different hashes for
/// different artifacts — never confused. `artifacts` lists every platform
/// artifact built from this logical image, each with its own hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedManifest {
    pub image_id: String,
    pub pegoles_image_version: String,
    pub debian_version: String,
    pub architecture: String,
    pub guest_runtime_version: String,
    pub guest_protocol_version: u32,
    pub source_image_sha512: String,
    pub image_sha512: String,
    pub built_at: String,
    /// Per-artifact records. The first entry is always this build's
    /// primary artifact (== `image_sha512`); later entries cover other
    /// platforms (e.g. a future VHDX) without reusing hashes.
    #[serde(default)]
    pub artifacts: Vec<ArtifactRecord>,
    /// Phase 4: graphical appliance components baked into this image.
    /// `None` = headless image (v0.1). Recorded by the builder from what
    /// it actually installed, never assumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphical: Option<GraphicalImageInfo>,
    /// Guest capabilities this image provides (Phase 5.1: `input`,
    /// `frame`). Empty for v0.1. Records image contents, not live
    /// handshake facts (the runtime still self-tests per connection).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// Graphical stack inside a derived image (names + versions as installed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicalImageInfo {
    /// e.g. "weston 14.0.1".
    pub compositor: String,
    /// e.g. "foot 1.21.0".
    pub terminal: String,
    /// e.g. "chromium 1xx" — `None` when deferred (Phase 4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
}

struct DerivedPaths {
    disk: PathBuf,
    marker: PathBuf,
    manifest: PathBuf,
}

/// Clone the sealed base into a private per-computer disk (APFS clone
/// on macOS via `fs::copy`). Source and destination must be regular
/// files; the copy is owner-only (0600) inside an owner-only dir.
fn instantiate_from(src_raw: &Path, dest_disk: &Path) -> Result<()> {
    require_regular_file(src_raw)?;
    if let Some(parent) = dest_disk.parent() {
        fs::create_dir_all(parent).map_err(|e| ComputerError::Backend(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    if fs::symlink_metadata(dest_disk).is_ok() {
        require_regular_file(dest_disk)?;
        // A leftover copy (an interrupted reset) may be read-only: Windows
        // refuses to overwrite it otherwise.
        make_writable(dest_disk)?;
    }
    fs::copy(src_raw, dest_disk).map_err(|e| ComputerError::Backend(e.to_string()))?;
    // The pinned base image is read-only and a copy keeps that on Windows;
    // the computer's own disk must be writable (the VM writes to it).
    make_writable(dest_disk)
}

/// Owner read/write only (Unix), or the read-only attribute cleared (Windows).
fn make_writable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| ComputerError::Backend(e.to_string()))
    }
    #[cfg(not(unix))]
    {
        let mut perms = fs::metadata(path)
            .map_err(|e| ComputerError::Backend(e.to_string()))?
            .permissions();
        // Windows: this only clears FILE_ATTRIBUTE_READONLY; access stays
        // governed by the per-user folder's ACL.
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(path, perms).map_err(|e| ComputerError::Backend(e.to_string()))
    }
}

/// Locate the largest non-EFI GPT partition (the Linux root) in a raw
/// disk image. Returns (byte offset, byte length). Pure parser over the
/// header + entry array; used by build tooling to inspect images without
/// mounting (e.g. kernel config checks via debugfs on the extracted
/// partition). Never modifies the image.
pub fn gpt_root_partition(raw_path: &Path) -> Result<(u64, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(raw_path).map_err(|e| ComputerError::Backend(e.to_string()))?;
    let mut hdr = [0u8; 512];
    f.seek(SeekFrom::Start(512))
        .and_then(|_| f.read_exact(&mut hdr))
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
    if &hdr[..8] != b"EFI PART" {
        return Err(ComputerError::Backend("not a GPT image".to_string()));
    }
    let le_u32 = |o: usize| u32::from_le_bytes(hdr[o..o + 4].try_into().expect("slice"));
    let le_u64 = |o: usize| u64::from_le_bytes(hdr[o..o + 8].try_into().expect("slice"));
    let entry_lba = le_u64(72);
    let entry_count = le_u32(80).min(256);
    let entry_size = le_u32(84).clamp(128, 512) as usize;
    // EFI System Partition type GUID (excluded from root candidacy).
    const ESP: [u8; 16] = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xd5,
        0x9b,
    ];
    let mut best: Option<(u64, u64)> = None;
    let mut entries = vec![0u8; entry_count as usize * entry_size];
    f.seek(SeekFrom::Start(entry_lba * 512))
        .and_then(|_| f.read_exact(&mut entries))
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
    for chunk in entries.chunks(entry_size) {
        if chunk.len() < 48 || chunk[..16].iter().all(|&b| b == 0) {
            continue;
        }
        if chunk[..16] == ESP {
            continue;
        }
        let first = u64::from_le_bytes(chunk[32..40].try_into().expect("slice"));
        let last = u64::from_le_bytes(chunk[40..48].try_into().expect("slice"));
        if last <= first {
            continue;
        }
        let len = (last - first + 1) * 512;
        if best.map(|(_, bl)| len > bl).unwrap_or(true) {
            best = Some((first * 512, len));
        }
    }
    best.ok_or_else(|| ComputerError::Backend("no root partition found".to_string()))
}

struct ImagePaths {
    dir: PathBuf,
    archive: PathBuf,
    partial: PathBuf,
    base_raw: PathBuf,
    marker: PathBuf,
}

/// Linux vsock transport support compiled into a guest kernel.
/// `BuiltIn` (y) and `Module` (m, with the .ko present) both count as
/// available; only `Missing` fails the image validation gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelConfigState {
    BuiltIn,
    Module,
    Missing,
}

/// Transports a Pegoles guest image must provide. Universal images need
/// BOTH virtio (macOS/KVM hosts) and Hyper-V (Windows hosts) support.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestKernelCapabilities {
    pub vsock: KernelConfigState,
    pub virtio_vsock: KernelConfigState,
    pub hyperv_vsock: KernelConfigState,
}

impl GuestKernelCapabilities {
    /// Ready for macOS/KVM hosts (virtio transport usable).
    pub fn virtio_ready(&self) -> bool {
        self.vsock != KernelConfigState::Missing && self.virtio_vsock != KernelConfigState::Missing
    }

    /// Ready for Windows/Hyper-V hosts.
    pub fn hyperv_ready(&self) -> bool {
        self.vsock != KernelConfigState::Missing && self.hyperv_vsock != KernelConfigState::Missing
    }

    /// Ready for every planned host (the universal-image gate).
    pub fn universal_ready(&self) -> bool {
        self.virtio_ready() && self.hyperv_ready()
    }
}

/// Parse a kernel config text (`/boot/config-*` or `/proc/config.gz`
/// contents) into transport capabilities. `=y` → BuiltIn, `=m` → Module,
/// `# … is not set` or absent → Missing. Pure and fully tested; the
/// builder feeds it real config text (see check-kernel.sh).
pub fn check_kernel_config(config_text: &str) -> GuestKernelCapabilities {
    fn state(config_text: &str, option: &str) -> KernelConfigState {
        for line in config_text.lines() {
            let line = line.trim();
            if line == format!("{option}=y") {
                return KernelConfigState::BuiltIn;
            }
            if line == format!("{option}=m") {
                return KernelConfigState::Module;
            }
            if line == format!("# {option} is not set") {
                return KernelConfigState::Missing;
            }
        }
        KernelConfigState::Missing
    }
    GuestKernelCapabilities {
        vsock: state(config_text, "CONFIG_VSOCKETS"),
        virtio_vsock: state(config_text, "CONFIG_VIRTIO_VSOCKETS"),
        hyperv_vsock: state(config_text, "CONFIG_HYPERV_VSOCKETS"),
    }
}

/// HTTPS-only agent with bounded redirects: a redirect can never
/// downgrade the official download to plain HTTP.
fn download_agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(5)
            .build(),
    )
}

fn http_get_text(url: &str) -> Result<String> {
    let mut res = download_agent()
        .get(url)
        .call()
        .map_err(|e| ComputerError::Backend(format!("download failed for {url}: {e}")))?;
    res.body_mut()
        .read_to_string()
        .map_err(|e| ComputerError::Backend(e.to_string()))
}

/// Parse a `SHA512SUMS` document, returning the hex digest for `artifact`.
pub fn parse_sha512sums(sums: &str, artifact: &str) -> Option<String> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let name = parts.next()?;
        let name = name.trim_start_matches('*').rsplit('/').next()?;
        if name == artifact && digest.len() == 128 && digest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Some(digest.to_lowercase());
        }
    }
    None
}

/// Streaming SHA-512 hex of a file (no full read into memory).
pub fn sha512_file(path: &Path) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| ComputerError::Backend(e.to_string()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn download_to(url: &str, dest_part: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    download_with(&download_agent(), url, dest_part, progress)
}

fn download_with(
    agent: &ureq::Agent,
    url: &str,
    dest_part: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut res = agent
        .get(url)
        .call()
        .map_err(|e| ComputerError::Backend(format!("download failed for {url}: {e}")))?;
    let total: u64 = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut out = fs::File::create(dest_part).map_err(|e| ComputerError::Backend(e.to_string()))?;
    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 128 * 1024];
    let mut body = res.body_mut().as_reader();
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        downloaded += n as u64;
        progress(downloaded, total);
    }
    out.flush()
        .map_err(|e| ComputerError::Backend(e.to_string()))?;
    Ok(())
}

fn extract_tar_xz(archive: &Path, dest_dir: &Path) -> Result<()> {
    // System bsdtar handles xz; this is first-party installer behavior
    // with fixed arguments. bsdtar refuses absolute and `..` member paths
    // by default; ownership/permission bits from the archive are ignored.
    let status = std::process::Command::new("/usr/bin/tar")
        .env_clear()
        .arg("-xJf")
        .arg(archive)
        .arg("--no-same-owner")
        .arg("--no-same-permissions")
        .arg("-C")
        .arg(dest_dir)
        .status()
        .map_err(|e| ComputerError::Backend(format!("failed to run /usr/bin/tar: {e}")))?;
    if !status.success() {
        return Err(ComputerError::Backend(
            "extracting verified image archive failed".to_string(),
        ));
    }
    Ok(())
}

/// The `.raw` member of an extracted archive. Regular files only: a
/// symlink member (e.g. pointing at a host secret) is never followed.
fn find_extracted_raw(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let regular = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if regular && path.extension().and_then(|e| e.to_str()) == Some("raw") {
            return Some(path);
        }
    }
    None
}

/// Refuse anything but a regular file (never follow a symlink).
fn require_regular_file(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| ComputerError::Backend(e.to_string()))?;
    if meta.file_type().is_file() {
        Ok(())
    } else {
        Err(ComputerError::ImageVerificationFailed(format!(
            "{} is not a regular file",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every host: the pinned base is read-only, the computer's copy must
    /// be writable (Windows keeps the read-only attribute across a copy),
    /// including over a read-only leftover from an interrupted attempt.
    #[test]
    fn a_computer_disk_copied_from_a_read_only_base_is_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("disk.vhdx");
        fs::write(&base, b"pinned base").unwrap();
        let mut ro = fs::metadata(&base).unwrap().permissions();
        ro.set_readonly(true);
        fs::set_permissions(&base, ro.clone()).unwrap();
        let dest = tmp.path().join("computers").join("c1").join("disk.vhdx");
        instantiate_from(&base, &dest).unwrap();
        assert!(!fs::metadata(&dest).unwrap().permissions().readonly());
        fs::write(&dest, b"guest writes").unwrap();
        fs::set_permissions(&dest, ro).unwrap();
        instantiate_from(&base, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"pinned base");
        assert!(!fs::metadata(&dest).unwrap().permissions().readonly());
    }

    const FIXTURE_SUMS: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f  debian-13-nocloud-arm64.tar.xz\n\
        da39a3ee5e6b4b0d3255bfef95601890afd80709  SHA512SUMS\n";

    #[test]
    fn parses_official_sums_format() {
        let got = parse_sha512sums(FIXTURE_SUMS, DEBIAN_ARTIFACT).expect("entry");
        assert!(got.starts_with("ddaf35a1"));
        assert_eq!(got.len(), 128);
    }

    #[test]
    fn missing_entry_returns_none() {
        assert!(parse_sha512sums(FIXTURE_SUMS, "nope.tar.xz").is_none());
        assert!(parse_sha512sums("garbage\n", DEBIAN_ARTIFACT).is_none());
    }

    #[test]
    fn sha512_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        fs::write(&f, b"abc").unwrap();
        // echo -n abc | shasum -a 512
        assert_eq!(
            sha512_file(&f).unwrap(),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn status_lifecycle_missing_downloading_invalid_ready() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        assert_eq!(mgr.status(), ImageStatus::Missing);
        assert!(mgr.load().is_err());

        // partial present -> Downloading
        let p = mgr.spec_paths(&PEGOLES_DEBIAN_13_ARM64);
        fs::create_dir_all(&p.dir).unwrap();
        fs::write(&p.partial, b"half").unwrap();
        assert_eq!(mgr.status(), ImageStatus::Downloading);

        // raw without marker -> Invalid, load fails closed
        fs::remove_file(&p.partial).unwrap();
        fs::write(&p.base_raw, b"unverified").unwrap();
        assert_eq!(mgr.status(), ImageStatus::Invalid);
        assert!(matches!(
            mgr.load(),
            Err(ComputerError::ImageVerificationFailed(_))
        ));

        // marker added -> Ready
        fs::write(&p.marker, "deadbeef").unwrap();
        assert_eq!(mgr.status(), ImageStatus::Ready);
        assert!(mgr.load().is_ok());
    }

    #[test]
    fn instantiate_copies_without_touching_base() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let p = mgr.spec_paths(&PEGOLES_DEBIAN_13_ARM64);
        fs::create_dir_all(&p.dir).unwrap();
        fs::write(&p.base_raw, b"base-bytes").unwrap();
        fs::write(&p.marker, "abc").unwrap();

        let dest = dir.path().join("vm1").join("disk.img");
        mgr.instantiate(&ComputerId::new(), &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"base-bytes");
        // base untouched
        assert_eq!(fs::read(&p.base_raw).unwrap(), b"base-bytes");
        // destination is writable even though the base is read-only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "instance disk: owner read-write only");
        }
    }

    #[test]
    fn download_streams_real_bytes_from_local_server() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let len = body.len();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut head);
            let response =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let mut seen = vec![];
        // Plain-HTTP agent for the loopback fixture only; production
        // downloads always go through the HTTPS-only agent.
        download_with(
            &ureq::Agent::new_with_defaults(),
            &format!("http://{addr}/file.bin"),
            &dest,
            &mut |d, t| seen.push((d, t)),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(&dest).unwrap().len(), len);
        let (last_d, last_t) = seen.last().copied().unwrap();
        assert_eq!(last_d as usize, len);
        assert_eq!(last_t as usize, len);
    }

    #[test]
    fn production_downloads_refuse_plain_http() {
        let dir = tempfile::tempdir().unwrap();
        let err = download_to(
            "http://127.0.0.1:9/never.bin",
            &dir.path().join("x"),
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(err, ComputerError::Backend(_)));
        assert!(!dir.path().join("x").exists());
    }

    #[cfg(unix)]
    #[test]
    fn instantiate_refuses_symlinked_base() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret");
        fs::write(&secret, b"host secret").unwrap();
        let link = dir.path().join("base.raw");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        let dest = dir.path().join("vm").join("disk.img");
        assert!(instantiate_from(&link, &dest).is_err());
        assert!(!dest.exists());
        // Extraction lookup never returns a symlinked .raw either.
        assert!(find_extracted_raw(dir.path()).is_none());
    }

    #[test]
    fn spec_points_only_at_official_debian() {
        for spec in [PEGOLES_DEBIAN_13_ARM64, GENERIC_DEBIAN_13_ARM64] {
            assert!(spec.base_url.starts_with("https://cdimage.debian.org/"));
            assert!(spec.artifact.ends_with("-arm64.tar.xz"));
        }
        assert_eq!(PEGOLES_DEBIAN_13_ARM64.id, "pegoles-debian-13-arm64");
        assert_eq!(
            PEGOLES_DEBIAN_13_ARM64.artifact,
            "debian-13-nocloud-arm64.tar.xz"
        );
        assert_eq!(
            GENERIC_DEBIAN_13_ARM64.artifact,
            "debian-13-generic-arm64.tar.xz"
        );
    }

    #[test]
    fn artifact_matrix_maps_format_per_platform() {
        use crate::platform::{DiskFormat, GuestArchitecture, HostPlatform};
        assert_eq!(
            disk_format_for(HostPlatform::MacOS, GuestArchitecture::Arm64),
            Some(DiskFormat::Raw)
        );
        assert_eq!(
            disk_format_for(HostPlatform::Windows, GuestArchitecture::X86_64),
            Some(DiskFormat::Vhdx)
        );
        // No silent defaults for unmapped pairs.
        assert_eq!(
            disk_format_for(HostPlatform::Linux, GuestArchitecture::Arm64),
            None
        );
        assert_eq!(
            disk_format_for(HostPlatform::MacOS, GuestArchitecture::X86_64),
            None
        );
        // Matrix self-consistency: file extensions match formats.
        for (artifact, _status) in known_artifacts() {
            assert!(
                artifact.file_name.ends_with(artifact.format.extension()),
                "mismatch: {} vs {:?}",
                artifact.file_name,
                artifact.format
            );
        }
    }

    #[test]
    fn manifest_hashes_never_shared_across_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let work = dir.path().join("work.raw");
        fs::write(&work, b"derived-bytes").unwrap();
        let manifest = mgr
            .publish_derived(
                &work,
                DerivedManifestInput::v0_1(
                    "13".into(),
                    "arm64".into(),
                    "0.1.0".into(),
                    1,
                    "sourcesha".into(),
                ),
            )
            .unwrap();
        assert_eq!(manifest.artifacts.len(), 1);
        assert_eq!(manifest.artifacts[0].sha512, manifest.image_sha512);
        assert_ne!(manifest.image_sha512, manifest.source_image_sha512);
    }

    #[test]
    fn kernel_config_builtin_module_missing() {
        let text =
            "CONFIG_VSOCKETS=y\nCONFIG_VIRTIO_VSOCKETS=m\n# CONFIG_HYPERV_VSOCKETS is not set\n";
        let caps = check_kernel_config(text);
        assert_eq!(caps.vsock, KernelConfigState::BuiltIn);
        assert_eq!(caps.virtio_vsock, KernelConfigState::Module);
        assert_eq!(caps.hyperv_vsock, KernelConfigState::Missing);
        assert!(caps.virtio_ready());
        assert!(!caps.hyperv_ready());
        assert!(!caps.universal_ready());
    }

    #[test]
    fn kernel_config_universal_ready() {
        let text = "CONFIG_VSOCKETS=y\nCONFIG_VIRTIO_VSOCKETS=y\nCONFIG_HYPERV_VSOCKETS=y\n";
        let caps = check_kernel_config(text);
        assert!(caps.universal_ready());
    }

    #[test]
    fn kernel_config_absent_means_missing() {
        let caps = check_kernel_config("# nothing here\n");
        assert_eq!(caps.vsock, KernelConfigState::Missing);
        assert!(!caps.universal_ready());
    }

    #[test]
    fn active_image_id_defaults_to_the_product_image() {
        let saved = std::env::var(IMAGE_ID_ENV).ok();
        std::env::remove_var(IMAGE_ID_ENV);
        assert_eq!(active_image_id(), PEGOLES_PRODUCT_IMAGE_ID);
        if !cfg!(debug_assertions) {
            // Release builds never honour the override.
            std::env::set_var(IMAGE_ID_ENV, PEGOLES_BASE_IMAGE_ID_V2);
            assert_eq!(active_image_id(), PEGOLES_PRODUCT_IMAGE_ID);
            match saved {
                Some(v) => std::env::set_var(IMAGE_ID_ENV, v),
                None => std::env::remove_var(IMAGE_ID_ENV),
            }
            return;
        }
        std::env::set_var(IMAGE_ID_ENV, PEGOLES_BASE_IMAGE_ID_V2);
        assert_eq!(active_image_id(), PEGOLES_BASE_IMAGE_ID_V2);
        // Unknown ids fail closed downstream (missing dir), never remap.
        std::env::set_var(IMAGE_ID_ENV, "pegoles-base-9.9");
        assert_eq!(active_image_id(), "pegoles-base-9.9");
        // Never a path.
        for evil in ["../../.ssh", "/etc", "a/b", ".."] {
            std::env::set_var(IMAGE_ID_ENV, evil);
            assert_eq!(active_image_id(), PEGOLES_PRODUCT_IMAGE_ID, "{evil}");
        }
        match saved {
            Some(v) => std::env::set_var(IMAGE_ID_ENV, v),
            None => std::env::remove_var(IMAGE_ID_ENV),
        }
    }

    #[test]
    fn v2_publish_records_graphical_and_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let work = dir.path().join("work.raw");
        fs::write(&work, b"v2-derived-bytes").unwrap();
        let manifest = mgr
            .publish_derived(
                &work,
                DerivedManifestInput {
                    image_id: PEGOLES_BASE_IMAGE_ID_V2.to_string(),
                    image_version: PEGOLES_IMAGE_VERSION_V2.to_string(),
                    debian_version: "13".into(),
                    architecture: "arm64".into(),
                    guest_runtime_version: "0.2.0".into(),
                    guest_protocol_version: 1,
                    source_image_sha512: "sourcesha".into(),
                    graphical: Some(GraphicalImageInfo {
                        compositor: "weston 14.0.2-1".into(),
                        terminal: "foot 1.21.0-2".into(),
                        browser: None,
                    }),
                    capabilities: vec!["input".into(), "frame".into()],
                },
            )
            .unwrap();
        assert_eq!(manifest.image_id, PEGOLES_BASE_IMAGE_ID_V2);
        assert_eq!(manifest.pegoles_image_version, PEGOLES_IMAGE_VERSION_V2);
        let g = manifest.graphical.expect("graphical recorded");
        assert_eq!(g.compositor, "weston 14.0.2-1");
        assert_eq!(
            manifest.capabilities,
            vec!["input".to_string(), "frame".to_string()]
        );
        // v0.1-style fresh publish stays headless with no caps.
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let work = dir.path().join("work.raw");
        fs::write(&work, b"v1-derived-bytes").unwrap();
        let manifest = mgr
            .publish_derived(
                &work,
                DerivedManifestInput::v0_1(
                    "13".into(),
                    "arm64".into(),
                    "0.1.0".into(),
                    1,
                    "sourcesha".into(),
                ),
            )
            .unwrap();
        assert_eq!(manifest.image_id, PEGOLES_BASE_IMAGE_ID);
        assert!(manifest.graphical.is_none());
        assert!(manifest.capabilities.is_empty());
    }

    #[test]
    fn reseal_same_file_updates_top_hash() {
        // Regression (Phase 5.1 reseal verification): republishing the
        // SAME file name must move the top-level hash to the new bytes,
        // or the manifest describes bytes that no longer exist.
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let work = dir.path().join("work.raw");
        fs::write(&work, b"v2-first-bytes").unwrap();
        let first = mgr
            .publish_derived(
                &work,
                DerivedManifestInput {
                    image_id: PEGOLES_BASE_IMAGE_ID_V2.to_string(),
                    image_version: PEGOLES_IMAGE_VERSION_V2.to_string(),
                    debian_version: "13".into(),
                    architecture: "arm64".into(),
                    guest_runtime_version: "0.1.0".into(),
                    guest_protocol_version: 1,
                    source_image_sha512: "sourcesha".into(),
                    graphical: None,
                    capabilities: Vec::new(),
                },
            )
            .unwrap();
        fs::write(&work, b"v2-fixed-bytes").unwrap();
        let second = mgr
            .publish_derived(
                &work,
                DerivedManifestInput {
                    image_id: PEGOLES_BASE_IMAGE_ID_V2.to_string(),
                    image_version: PEGOLES_IMAGE_VERSION_V2.to_string(),
                    debian_version: "13".into(),
                    architecture: "arm64".into(),
                    guest_runtime_version: "0.1.0".into(),
                    guest_protocol_version: 1,
                    source_image_sha512: "sourcesha".into(),
                    graphical: None,
                    capabilities: Vec::new(),
                },
            )
            .unwrap();
        assert_ne!(first.image_sha512, second.image_sha512);
        assert_eq!(second.artifacts.len(), 1);
        assert_eq!(second.artifacts[0].sha512, second.image_sha512);
    }

    #[test]
    fn seed_iso_skips_derived_preference_for_builders() {
        // Regression (Phase 5.1 hardware): with a Ready derived image,
        // normal boots use it — but a builder holding PEGOLES_SEED_ISO
        // must boot the official cloud image instead, or it clones an
        // already-provisioned disk whose cloud-init never runs again.
        let dir = tempfile::tempdir().unwrap();
        let mgr =
            ComputerImageManager::with_spec(dir.path().to_path_buf(), GENERIC_DEBIAN_13_ARM64);
        let work = dir.path().join("work.raw");
        fs::write(&work, b"derived-bytes").unwrap();
        mgr.publish_derived(
            &work,
            DerivedManifestInput::v0_1(
                "13".into(),
                "arm64".into(),
                "0.1.0".into(),
                1,
                "sourcesha".into(),
            )
            .for_active_image(),
        )
        .unwrap();
        assert_eq!(mgr.derived_status(), ImageStatus::Ready);
        let saved_seed = std::env::var("PEGOLES_SEED_ISO").ok();
        std::env::remove_var("PEGOLES_SEED_ISO");
        let normal = mgr.boot_source_for_format_with_fallback(DiskFormat::Raw, true);
        assert!(matches!(normal, Ok(BootSource::Derived(_))));
        std::env::set_var("PEGOLES_SEED_ISO", "/tmp/does-not-need-to-exist.iso");
        let building = mgr.boot_source_for_format_with_fallback(DiskFormat::Raw, true);
        if cfg!(debug_assertions) {
            // No official artifact cached in this temp dir → fails closed on
            // the official path, but crucially NOT Derived.
            assert!(
                !matches!(building, Ok(BootSource::Derived(_))),
                "builder must not clone the derived image"
            );
        } else {
            // Release builds have no builder mode: the env var is ignored.
            assert!(matches!(building, Ok(BootSource::Derived(_))));
        }
        match saved_seed {
            Some(v) => std::env::set_var("PEGOLES_SEED_ISO", v),
            None => std::env::remove_var("PEGOLES_SEED_ISO"),
        }
    }

    /// A sealed image that is not the pinned release image is `Invalid` and
    /// never cloned when pins are enforced (release builds), whatever its
    /// own manifest and marker say.
    #[test]
    fn enforced_pins_reject_an_unpinned_sealed_image() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        let work = dir.path().join("work.raw");
        fs::write(&work, b"self-consistent but not the release").unwrap();
        let mut input = DerivedManifestInput::v0_1(
            "13".into(),
            "arm64".into(),
            "0.2.0".into(),
            1,
            "sourcesha".into(),
        );
        input.image_id = PEGOLES_PRODUCT_IMAGE_ID.to_string();
        mgr.publish_derived(&work, input).unwrap();
        let id = PEGOLES_PRODUCT_IMAGE_ID;
        assert_eq!(mgr.derived_status_with(id, false), ImageStatus::Ready);
        assert_eq!(mgr.derived_status_with(id, true), ImageStatus::Invalid);
        assert!(matches!(
            mgr.verify_pinned(id),
            Err(ComputerError::ImageVerificationFailed(_))
        ));
        // (No env changes here: other tests read the image id concurrently.)
        let dest = dir.path().join("vm").join("disk.img");
        let refused = mgr.instantiate_boot_source_with(
            &ComputerId::new(),
            &dest,
            DiskFormat::Raw,
            false,
            true,
        );
        assert!(refused.is_err(), "an unpinned image must not be cloned");
        assert!(!dest.exists(), "nothing is cloned from an unpinned image");
        // An id with no pin at all is refused too.
        assert!(mgr.verify_pinned("pegoles-base-9.9").is_err());
    }

    #[test]
    fn gpt_parser_finds_largest_non_efi_partition() {
        // Synthetic GPT: header + 4 entries (empty, ESP 128MiB, root 2GiB,
        // data 512MiB). Parser must skip ESP/empty and pick root.
        fn entry(type_guid: [u8; 16], first: u64, last: u64) -> Vec<u8> {
            let mut e = vec![0u8; 128];
            e[..16].copy_from_slice(&type_guid);
            e[32..40].copy_from_slice(&first.to_le_bytes());
            e[40..48].copy_from_slice(&last.to_le_bytes());
            e
        }
        let linux_fs = [0xafu8; 16];
        let esp = [
            0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
            0xd5, 0x9b,
        ];
        let mut img = vec![0u8; 512 + 512 + 4 * 128];
        img[512..520].copy_from_slice(b"EFI PART");
        img[512 + 72..512 + 80].copy_from_slice(&2u64.to_le_bytes());
        img[512 + 80..512 + 84].copy_from_slice(&4u32.to_le_bytes());
        img[512 + 84..512 + 88].copy_from_slice(&128u32.to_le_bytes());
        let mut off = 1024;
        for e in [
            vec![0u8; 128],
            entry(esp, 2048, 2048 + 262143),
            entry(linux_fs, 262144, 262144 + 4194303),
            entry(linux_fs, 4456448, 4456448 + 1048575),
        ] {
            img[off..off + 128].copy_from_slice(&e);
            off += 128;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.raw");
        fs::write(&path, &img).unwrap();
        let (offset, len) = gpt_root_partition(&path).unwrap();
        assert_eq!(offset, 262144 * 512);
        assert_eq!(len, 4194304 * 512);
    }

    #[test]
    fn gpt_parser_rejects_non_gpt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.img");
        fs::write(&path, vec![0u8; 2048]).unwrap();
        assert!(gpt_root_partition(&path).is_err());
    }

    #[test]
    fn derived_publish_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ComputerImageManager::new(dir.path().to_path_buf());
        assert_eq!(mgr.derived_status(), ImageStatus::Missing);
        assert!(mgr.derived_manifest().is_err());

        let work = dir.path().join("work.raw");
        fs::write(&work, b"derived-bytes").unwrap();
        let manifest = mgr
            .publish_derived(
                &work,
                DerivedManifestInput::v0_1(
                    "13".into(),
                    "arm64".into(),
                    "0.1.0".into(),
                    1,
                    "sourcesha".into(),
                )
                .for_active_image(),
            )
            .unwrap();
        assert_eq!(manifest.image_id, active_image_id());
        assert_eq!(manifest.source_image_sha512, "sourcesha");
        assert_ne!(manifest.image_sha512, manifest.source_image_sha512);
        assert_eq!(manifest.image_sha512.len(), 128);
        assert_eq!(mgr.derived_status(), ImageStatus::Ready);
        assert_eq!(mgr.derived_manifest().unwrap(), manifest);

        // Per-computer copies come from the derived image.
        let dest = dir.path().join("vm9").join("disk.img");
        mgr.instantiate_derived(&ComputerId::new(), &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"derived-bytes");

        // boot_source prefers derived over official.
        let official_dir = dir.path().join("images2");
        let mgr2 = ComputerImageManager::new(official_dir);
        // (mgr has no official image and a derived one -> Derived)
        match mgr.boot_source().unwrap() {
            BootSource::Derived(p) => assert!(p.ends_with("disk.raw")),
            BootSource::Official(_) => panic!("expected derived"),
        }
        let _ = mgr2;
    }
}

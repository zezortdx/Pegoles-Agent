//! VM resource validation.
//!
//! `ComputerConfig` itself lives in `pegoles-protocol` (types only, no logic).
//! All bounds are enforced here so the UI, registry, and backends share one
//! source of truth. Users cannot request absurd values.

use pegoles_protocol::ComputerConfig;

use crate::error::{ComputerError, Result};

/// Default for Phase 2: 2 vCPU / 1536 MB RAM.
/// 1536 MB is the smallest value we consider reliable for a Debian 13
/// minimal console guest (cloud images document ~1 GB minimum; the extra
/// headroom covers kernel + systemd + serial console without waste).
/// Disk stays nominal at 20 GB; the instance disk is a copy of the ~3 GB
/// base raw, grown only if a later phase needs it.
pub fn default_config() -> ComputerConfig {
    ComputerConfig {
        vcpus: 2,
        memory_mb: 1536,
        disk_gb: 20,
        workspace_root: pegoles_protocol::VirtualPath::new("/home/pegoles/workspace"),
        display: None,
    }
}

pub const MIN_VCPUS: u8 = 1;
pub const MAX_VCPUS_HARD: u8 = 8;
pub const MIN_RAM_MB: u32 = 512;
pub const MAX_RAM_MB: u32 = 8192;
pub const MIN_DISK_GB: u32 = 4;
pub const MAX_DISK_GB: u32 = 64;

/// Upper CPU bound follows the host: never promise more vCPUs than the
/// host can schedule.
pub fn max_vcpus_for_host() -> u8 {
    let host = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (host as u8).clamp(MIN_VCPUS, MAX_VCPUS_HARD)
}

pub fn validate_config(cfg: &ComputerConfig) -> Result<()> {
    if cfg.vcpus < MIN_VCPUS || cfg.vcpus > max_vcpus_for_host() {
        return Err(ComputerError::InvalidConfig(format!(
            "vcpus must be {}..={}, got {}",
            MIN_VCPUS,
            max_vcpus_for_host(),
            cfg.vcpus
        )));
    }
    if cfg.memory_mb < MIN_RAM_MB || cfg.memory_mb > MAX_RAM_MB {
        return Err(ComputerError::InvalidConfig(format!(
            "memory_mb must be {MIN_RAM_MB}..={MAX_RAM_MB}, got {}",
            cfg.memory_mb
        )));
    }
    if cfg.disk_gb < MIN_DISK_GB || cfg.disk_gb > MAX_DISK_GB {
        return Err(ComputerError::InvalidConfig(format!(
            "disk_gb must be {MIN_DISK_GB}..={MAX_DISK_GB}, got {}",
            cfg.disk_gb
        )));
    }
    if cfg.workspace_root.as_str().is_empty() || !cfg.workspace_root.as_str().starts_with('/') {
        return Err(ComputerError::InvalidConfig(
            "workspace_root must be an absolute guest path".to_string(),
        ));
    }
    if let Some(display) = cfg.display {
        // Headless (None) is always valid; a framebuffer must pass the
        // shared bounds/shape rules (the native layer re-validates).
        display.validate().map_err(|e| {
            ComputerError::InvalidConfig(format!(
                "display {}x{} ({:?}) rejected: {e:?}",
                display.width_px, display.height_px, display.profile
            ))
        })?;
    }
    Ok(())
}

/// Override for the Pegoles data root (tests, custom installs).
pub const DATA_DIR_ENV: &str = "PEGOLES_DATA_DIR";

/// Root for all Pegoles host-side data (see `platform::PegolesPaths`).
/// Thin wrapper kept so existing callers don't churn.
pub fn pegoles_data_dir() -> std::path::PathBuf {
    crate::platform::PegolesPaths::for_current_host()
        .root()
        .clone()
}
/// Whether this process can drive a real VM (macOS Apple Silicon only).
pub fn is_real_backend_supported() -> bool {
    cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")
}

pub fn require_real_backend_supported() -> Result<()> {
    if is_real_backend_supported() {
        Ok(())
    } else {
        Err(ComputerError::UnsupportedPlatform(format!(
            "{}-{}: real Pegoles Computer requires macOS on Apple Silicon (arm64)",
            std::env::consts::OS,
            std::env::consts::ARCH
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        validate_config(&default_config()).expect("default must be valid");
    }

    #[test]
    fn rejects_absurd_values() {
        let mut cfg = default_config();
        cfg.vcpus = 0;
        assert!(validate_config(&cfg).is_err());
        cfg = default_config();
        cfg.vcpus = 64;
        assert!(validate_config(&cfg).is_err());
        cfg = default_config();
        cfg.memory_mb = 128;
        assert!(validate_config(&cfg).is_err());
        cfg = default_config();
        cfg.memory_mb = 1 << 20;
        assert!(validate_config(&cfg).is_err());
        cfg = default_config();
        cfg.disk_gb = 1;
        assert!(validate_config(&cfg).is_err());
    }

    #[test]
    fn display_is_validated() {
        use pegoles_protocol::{DisplayConfig, DisplayProfile};
        let mut cfg = default_config();
        assert_eq!(cfg.display, None, "default stays headless");
        cfg.display = DisplayConfig::for_profile(DisplayProfile::DesktopLarge);
        validate_config(&cfg).expect("desktop preset is valid");
        for bad in [
            DisplayConfig::custom(320, 240),
            DisplayConfig::custom(1441, 900),
            DisplayConfig::custom(9000, 4000),
            DisplayConfig::for_profile(DisplayProfile::MobileRemote).unwrap(),
        ] {
            cfg.display = Some(bad);
            assert!(
                matches!(validate_config(&cfg), Err(ComputerError::InvalidConfig(_))),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn max_vcpus_respects_host() {
        let max = max_vcpus_for_host();
        assert!((MIN_VCPUS..=MAX_VCPUS_HARD).contains(&max));
    }
}

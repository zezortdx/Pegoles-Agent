//! Display, graphical-session and control-ownership vocabulary (Phase 4).
//!
//! Platform-free by design: these types describe WHAT the computer's
//! screen is doing and WHO is driving it, never HOW a hypervisor draws
//! it. Native display adapters live outside the shared crates.
//!
//! SECURITY: `ControlOwner::User` means a HUMAN is sending input to the
//! isolated guest through the host window. `ControlOwner::Agent` means a
//! deterministic Phase 5 sequence is driving structured actions that
//! passed Pegoles Policy — set only by the Core executor, never by the
//! UI and never by a model directly. Nothing in this module lets anyone
//! synthesize host input.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ComputerId, FrameId};

/// Named display presets. Resolution is data, never hardcoded in callers:
/// remote profiles are reserved for later phases (no streaming exists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayProfile {
    /// Default local desktop viewing (1440x900).
    DesktopLarge,
    /// Smaller hosts / split layouts (1280x800).
    DesktopCompact,
    /// Reserved: future phone viewing over Pegoles Link.
    MobileRemote,
    /// Reserved: future constrained-bandwidth viewing.
    LowBandwidthRemote,
    /// Explicit dimensions (validated like every other profile).
    Custom,
}

impl DisplayProfile {
    /// Whether this profile needs remote streaming (not built yet).
    pub fn is_remote(&self) -> bool {
        matches!(
            self,
            DisplayProfile::MobileRemote | DisplayProfile::LowBandwidthRemote
        )
    }
}

/// Guest framebuffer configuration, fixed at computer creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub profile: DisplayProfile,
    pub width_px: u32,
    pub height_px: u32,
}

/// Why a display configuration was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayConfigError {
    TooSmall,
    TooLarge,
    /// Odd pixel counts break common scanout/encoder alignment.
    OddDimension,
    /// Aspect ratio outside 1:2 .. 2.4:1.
    ExtremeAspect,
    /// Remote profiles need streaming, which does not exist yet.
    RemoteProfileUnavailable,
}

impl DisplayConfig {
    pub const MIN_WIDTH_PX: u32 = 640;
    pub const MIN_HEIGHT_PX: u32 = 480;
    pub const MAX_WIDTH_PX: u32 = 3840;
    pub const MAX_HEIGHT_PX: u32 = 2400;

    /// Preset for a profile. `Custom` has no preset (use `custom`).
    pub fn for_profile(profile: DisplayProfile) -> Option<Self> {
        let (w, h) = match profile {
            DisplayProfile::DesktopLarge => (1440, 900),
            DisplayProfile::DesktopCompact => (1280, 800),
            DisplayProfile::MobileRemote => (1170, 2532),
            DisplayProfile::LowBandwidthRemote => (1024, 640),
            DisplayProfile::Custom => return None,
        };
        Some(Self {
            profile,
            width_px: w,
            height_px: h,
        })
    }

    pub fn custom(width_px: u32, height_px: u32) -> Self {
        Self {
            profile: DisplayProfile::Custom,
            width_px,
            height_px,
        }
    }

    /// Bounds + shape validation. Callers must validate before creating a
    /// computer; the native layer re-validates (defense in depth).
    pub fn validate(&self) -> Result<(), DisplayConfigError> {
        if self.profile.is_remote() {
            return Err(DisplayConfigError::RemoteProfileUnavailable);
        }
        if self.width_px < Self::MIN_WIDTH_PX || self.height_px < Self::MIN_HEIGHT_PX {
            return Err(DisplayConfigError::TooSmall);
        }
        if self.width_px > Self::MAX_WIDTH_PX || self.height_px > Self::MAX_HEIGHT_PX {
            return Err(DisplayConfigError::TooLarge);
        }
        if !self.width_px.is_multiple_of(2) || !self.height_px.is_multiple_of(2) {
            return Err(DisplayConfigError::OddDimension);
        }
        let aspect = self.aspect_ratio();
        if !(0.5..=2.4).contains(&aspect) {
            return Err(DisplayConfigError::ExtremeAspect);
        }
        Ok(())
    }

    pub fn aspect_ratio(&self) -> f64 {
        self.width_px as f64 / self.height_px.max(1) as f64
    }
}

/// Guest-reported state of the graphical session (compositor). Distinct
/// from `GuestRuntimeState::Ready`: the control plane can be ready while
/// the compositor is still starting (or absent on a headless image).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicalSessionState {
    /// No graphical session on this computer (headless image, no display
    /// device, or not reported yet by an older guest runtime).
    Unavailable,
    /// Compositor launching; not accepting clients yet.
    Starting,
    /// Compositor answered a real client round-trip.
    Ready,
    /// Compositor failed or exited.
    Failed,
}

/// Who is driving Pegoles Computer's input right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOwner {
    /// Nobody: the display is view-only.
    #[default]
    None,
    /// A deterministic agent sequence owns input (Phase 5 executor only:
    /// policy-checked structured actions over the guest channel).
    Agent,
    /// A human took control from the host window.
    User,
}

/// What the ComputerViewport shows. Derived by Pegoles Core from real
/// backend facts (computer state, guest state, graphical session, display
/// attachment, control owner) — never guessed by the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewportState {
    Off,
    Preparing,
    Starting,
    GuestConnecting,
    DisplayStarting,
    Ready,
    Paused,
    AgentActive,
    UserControlled,
    Error,
}

/// Pixel encoding of an observed guest frame. The protocol is NOT bound
/// to one encoding: PNG for manual debugging, raw RGBA for model
/// perception pipelines that decode themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameEncoding {
    /// Raw 8-bit RGBA, row-major, top-left origin, no stride padding.
    #[default]
    RawRgba,
    /// PNG-encoded image bytes.
    Png,
}

impl FrameEncoding {
    /// Bytes per pixel for encodings with a fixed stride.
    pub fn bytes_per_pixel(self) -> Option<u64> {
        match self {
            FrameEncoding::RawRgba => Some(4),
            FrameEncoding::Png => None,
        }
    }
}

/// Identity + metadata of one captured guest frame. Pixels travel
/// out-of-band (Tauri command return / dev panel), never in events:
/// events carry this metadata only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedFrameMeta {
    pub frame_id: FrameId,
    pub computer_id: ComputerId,
    pub captured_at: DateTime<Utc>,
    pub width_px: u32,
    pub height_px: u32,
    pub encoding: FrameEncoding,
    /// Byte length of the encoded payload.
    pub byte_len: u64,
    /// Milliseconds from capture request to bytes ready.
    pub capture_latency_ms: u64,
}

impl ObservedFrameMeta {
    /// Sanity check against the encoding's fixed stride (when known).
    pub fn stride_consistent(&self) -> bool {
        match self.encoding.bytes_per_pixel() {
            Some(bpp) => self.byte_len == self.width_px as u64 * self.height_px as u64 * bpp,
            None => self.byte_len > 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_validate() {
        for p in [DisplayProfile::DesktopLarge, DisplayProfile::DesktopCompact] {
            let cfg = DisplayConfig::for_profile(p).unwrap();
            assert_eq!(cfg.validate(), Ok(()));
        }
        assert_eq!(DisplayConfig::for_profile(DisplayProfile::Custom), None);
    }

    #[test]
    fn remote_profiles_are_reserved() {
        let cfg = DisplayConfig::for_profile(DisplayProfile::MobileRemote).unwrap();
        assert_eq!(
            cfg.validate(),
            Err(DisplayConfigError::RemoteProfileUnavailable)
        );
    }

    #[test]
    fn invalid_dimensions_rejected() {
        assert_eq!(
            DisplayConfig::custom(320, 240).validate(),
            Err(DisplayConfigError::TooSmall)
        );
        assert_eq!(
            DisplayConfig::custom(7680, 4320).validate(),
            Err(DisplayConfigError::TooLarge)
        );
        assert_eq!(
            DisplayConfig::custom(1441, 900).validate(),
            Err(DisplayConfigError::OddDimension)
        );
        assert_eq!(
            DisplayConfig::custom(3840, 640).validate(),
            Err(DisplayConfigError::ExtremeAspect)
        );
    }

    #[test]
    fn wire_names_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&ViewportState::GuestConnecting).unwrap(),
            "\"guest_connecting\""
        );
        assert_eq!(
            serde_json::to_string(&ControlOwner::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&GraphicalSessionState::Ready).unwrap(),
            "\"ready\""
        );
    }
}

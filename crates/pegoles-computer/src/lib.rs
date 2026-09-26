//! Pegoles Computer abstraction.
//!
//! `pegoles-core` depends only on [`ComputerBackend`], never on a concrete
//! virtualization API. Future backends:
//! `MacOSVirtualizationBackend`, `LinuxKvmBackend`, `WindowsHyperVBackend`.
//! Phase 1 ships [`MockComputerBackend`] for development and tests.

pub mod computer_store;
pub mod config;
pub mod coords;
pub mod display;
pub mod error;
pub mod governor;
pub mod guest;
pub mod image;
pub mod input;
pub mod macos;
pub mod mock;
pub mod native_backend;
pub mod platform;
pub mod traits;
pub mod transport;
pub mod vmhost_proto;
pub mod windows;

pub use config::{
    default_config, is_real_backend_supported, pegoles_data_dir, require_real_backend_supported,
    validate_config, DATA_DIR_ENV,
};
pub use coords::{AgentPoint, DisplayTransform, GuestPoint, NativeViewport};
pub use display::{
    ComputerDisplayBackend, DisplayAttachment, DisplayBackendKind, DisplayEvent, DisplayGeometry,
    DisplayPresentation, DisplayRect, GeometryError, TestDisplay, TestDisplayCall,
    TestDisplayState, UnavailableDisplay, MAX_GEOMETRY_ANIMATION_MS,
};
pub use error::{ComputerError, Result};
pub use governor::{
    detect_host_resources, recommend, recommend_effects, BalloonPolicy, ComputerRecommendation,
    EffectsReason, EffectsRecommendation, EffectsTier, HostResources, IdlePolicy, IdleState,
    MemoryPressure, PerformanceProfile, PowerSource,
};
pub use guest::{
    GraphicalSessionChange, GraphicalSessionInfo, GuestObservation, GuestSession, Outbound,
    SessionOutcome, GUEST_READY_TIMEOUT, HEARTBEAT_INTERVAL, HEARTBEAT_MISS_LIMIT,
};
pub use image::{
    active_image_id, check_kernel_config, disk_format_for, gpt_root_partition, known_artifacts,
    ArtifactRecord, ArtifactStatus, BootSource, ComputerImage, ComputerImageManager,
    DerivedManifest, DerivedManifestInput, GraphicalImageInfo, GuestKernelCapabilities,
    ImageFamily, ImageSpec, ImageStatus, KernelConfigState, PlatformArtifact, PrepareStage,
    GENERIC_DEBIAN_13_AMD64, GENERIC_DEBIAN_13_ARM64, IMAGE_ID_ENV, PEGOLES_BASE_IMAGE_ID,
    PEGOLES_BASE_IMAGE_ID_V2, PEGOLES_BASE_IMAGE_ID_V3, PEGOLES_IMAGE_VERSION,
    PEGOLES_IMAGE_VERSION_V2, PEGOLES_IMAGE_VERSION_V3, PEGOLES_PRODUCT_IMAGE_ID,
};
pub use input::{
    action_to_input_ops, base64_png, chunk_bytes, drag_step_count, encode_png_rgb_fast,
    encode_png_rgba, op_to_guest, reassemble_chunks, ActionRateLimiter, CapturedFrame,
    ComputerInputBackend, FrameCache, InputBackendKind, InputCapabilities, InputOp, InputOutcome,
    PressedState, TestInput, UnavailableInput, WindowsInputStub,
};
pub use macos::MacOSVirtualizationBackend;
pub use mock::MockComputerBackend;
pub use pegoles_guest_proto::MAX_FRAME_BYTES as GUEST_FRAME_MAX_BYTES;
pub use pegoles_guest_proto::{CapabilityDiagnostic, GUEST_CAP_FRAME, GUEST_CAP_INPUT};
pub use platform::{
    host_architecture, host_capabilities, host_platform, BackendCapabilities, BackendKind,
    DiskFormat, GuestArchitecture, GuestTransportKind, HostArchitecture, HostCapabilities,
    HostPlatform, PegolesPaths,
};
pub use traits::{ComputerBackend, ComputerInstance};
pub use transport::{FakeGuestTransport, GuestTransport, TransportEvent};
pub use windows::{
    hyperv_service_guid_for_port, pegoles_hyperv_service_guid, probe_windows_host,
    WindowsHcsBackend, WindowsHostCapabilities, WindowsSupport, HV_SOCKET_REGISTRY_PATH,
};

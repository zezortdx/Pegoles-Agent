//! Seal an x64 image built by `scripts/build-guest-image/build-x64.sh` as
//! the Windows product image (`PEGOLES_BASE_IMAGE_ID_X64`) in this user's
//! Pegoles data folder. BUILD/TEST TIME ONLY.
//!
//! Hashes the bytes on disk and records the facts from the build's
//! `manifest.json` (never invents any). Release builds boot only the
//! image pinned in `catalog/images.json`, so a locally sealed image is used
//! by debug builds (CI's Windows agent E2E, development).
//!
//! Usage:
//!   cargo run -p pegoles-computer --example seal_x64_image -- <disk.vhdx> <manifest.json>

use pegoles_computer::{
    pegoles_data_dir, ComputerImageManager, DerivedManifestInput, DiskFormat, GraphicalImageInfo,
    PEGOLES_BASE_IMAGE_ID_X64,
};
use pegoles_guest_proto::GUEST_PROTOCOL_VERSION;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(disk), Some(manifest)) = (args.first(), args.get(1)) else {
        eprintln!("usage: seal_x64_image <disk.vhdx> <manifest.json>");
        std::process::exit(2);
    };
    let built: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).expect("read manifest"))
            .expect("manifest is JSON");
    let text = |pointer: &str| {
        built
            .pointer(pointer)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("manifest has no {pointer}"))
            .to_string()
    };
    assert_eq!(text("/architecture"), "amd64", "not an x64 image");
    let source = text("/source/sha512");
    assert!(
        source.len() == 128 && source.chars().all(|c| c.is_ascii_hexdigit()),
        "source digest is not SHA-512"
    );
    let input = DerivedManifestInput {
        image_id: PEGOLES_BASE_IMAGE_ID_X64.to_string(),
        image_version: "x64-0.1".into(),
        debian_version: text("/debian_version"),
        architecture: "amd64".into(),
        guest_runtime_version: text("/guest_runtime_version"),
        guest_protocol_version: GUEST_PROTOCOL_VERSION,
        source_image_sha512: source,
        graphical: Some(GraphicalImageInfo {
            compositor: text("/graphical/compositor"),
            terminal: text("/graphical/terminal"),
            browser: None,
        }),
        capabilities: vec!["input".to_string(), "frame".to_string()],
    };
    let mgr = ComputerImageManager::new(pegoles_data_dir().join("images"));
    let sealed = mgr
        .publish_derived_as(
            std::path::Path::new(disk),
            input,
            "disk.vhdx",
            DiskFormat::Vhdx,
        )
        .expect("publish the x64 image");
    println!(
        "sealed {PEGOLES_BASE_IMAGE_ID_X64}:\n{}",
        serde_json::to_string_pretty(&sealed).unwrap()
    );
}

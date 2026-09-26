//! Build-time provisioner for Pegoles Guest Image v2 (graphical +
//! input/capture). Mirrors `provision_base_image` but seals a NEW
//! versioned image (`pegoles-base-0.2`); v0.1 is never touched.
//!
//! BUILD TIME ONLY. The normal lifecycle never sets PEGOLES_IMAGE_SPEC
//! or PEGOLES_SEED_ISO.
//!
//! Usage:
//!   PEGOLES_SEED_ISO=/tmp/pgv2/seed-v2.iso PEGOLES_DEBS_DIR=/tmp/pgv2-debs \
//!     PEGOLES_IMAGE_SPEC=generic \
//!     cargo run -p pegoles-computer --example provision_v2_image
//!
//! Graphical versions are read from $PEGOLES_DEBS_DIR/VERSIONS.txt (the
//! exact debs staged on the seed ISO) — recorded, never assumed. When
//! VERSIONS.txt is absent the manifest seals with `graphical: None`
//! and a warning (headless fallback, still versioned v0.2).

use pegoles_computer::{
    default_config, pegoles_data_dir, ComputerBackend, ComputerImageManager, DerivedManifestInput,
    GraphicalImageInfo, MacOSVirtualizationBackend, GENERIC_DEBIAN_13_ARM64,
    PEGOLES_BASE_IMAGE_ID_V3, PEGOLES_IMAGE_VERSION_V3,
};
use pegoles_guest_proto::{GUEST_PROTOCOL_VERSION, RUNTIME_VERSION};
use std::time::{Duration, Instant};

/// `Package: weston Version: 14.0.2-1 ...` lines → ("weston", "14.0.2-1").
fn pinned_version(versions_txt: &str, package: &str) -> Option<String> {
    for line in versions_txt.lines() {
        let mut parts = line.split_whitespace();
        if parts.next()? != "Package:" {
            continue;
        }
        let name = parts.next()?;
        if name != package {
            continue;
        }
        // Expect `Version: <v> Architecture: <arch>`.
        let rest: Vec<&str> = parts.collect();
        if let Some(i) = rest.iter().position(|t| *t == "Version:") {
            if let Some(v) = rest.get(i + 1) {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn graphical_from_bundle() -> Option<GraphicalImageInfo> {
    let dir = std::env::var("PEGOLES_DEBS_DIR").ok()?;
    let text = std::fs::read_to_string(format!("{dir}/VERSIONS.txt")).ok()?;
    let weston = pinned_version(&text, "weston")?;
    let foot = pinned_version(&text, "foot")?;
    Some(GraphicalImageInfo {
        compositor: format!("weston {weston}"),
        terminal: format!("foot {foot}"),
        browser: None,
    })
}

fn main() {
    let data = pegoles_data_dir();
    println!("data dir: {}", data.display());

    // 1. Official generic image, verified like everything else.
    let generic = ComputerImageManager::with_spec(data.join("images"), GENERIC_DEBIAN_13_ARM64);
    println!("generic image status: {:?}", generic.status());
    let source = generic
        .prepare(&mut |stage, downloaded, total| {
            eprintln!("generic: {stage:?} {downloaded}/{total}");
        })
        .expect("prepare generic image");
    println!("generic ready: {}", source.base_raw.display());

    // 2. Boot it once with the v2 seed ISO attached.
    let mut backend = MacOSVirtualizationBackend::new(data.join("images"), data.join("computers"))
        .expect("macos backend");
    let id = backend.create(default_config()).expect("create");
    println!("build computer: {id}");
    backend.start().expect("start");

    // 3. Wait for cloud-init to finish and power the VM off.
    let deadline = Instant::now() + Duration::from_secs(20 * 60);
    loop {
        if Instant::now() >= deadline {
            eprintln!("TIMEOUT waiting for provisioning poweroff");
            std::process::exit(1);
        }
        let state = backend.state();
        println!("provision state: {state:?}");
        if state == pegoles_protocol::ComputerState::Stopped {
            break;
        }
        std::thread::sleep(Duration::from_secs(10));
    }

    // 4. Seal the disk as the v2 derived image (hashes computed, not invented).
    let graphical = graphical_from_bundle();
    match &graphical {
        None => eprintln!("WARNING: no VERSIONS.txt graphical facts; sealing headless v0.2"),
        Some(g) => println!("graphical: {g:?}"),
    }
    let disk = data.join("computers").join(id.to_string()).join("disk.img");
    let mgr = ComputerImageManager::new(data.join("images"));
    let input = DerivedManifestInput {
        image_id: PEGOLES_BASE_IMAGE_ID_V3.to_string(),
        image_version: PEGOLES_IMAGE_VERSION_V3.to_string(),
        debian_version: "13".into(),
        architecture: "arm64".into(),
        guest_runtime_version: RUNTIME_VERSION.into(),
        guest_protocol_version: GUEST_PROTOCOL_VERSION,
        source_image_sha512: source.sha512.clone(),
        graphical,
        capabilities: vec!["input".to_string(), "frame".to_string()],
    };
    let manifest = mgr
        .publish_derived(&disk, input)
        .expect("publish derived v2");
    println!(
        "derived manifest:\n{}",
        serde_json::to_string_pretty(&manifest).unwrap()
    );

    // 5. Drop the build computer (helper child dies with the backend).
    std::fs::remove_dir_all(data.join("computers").join(id.to_string())).ok();
    println!("DONE: images/pegoles-base-0.2 is sealed");
}

#[cfg(test)]
mod tests {
    use super::pinned_version;

    #[test]
    fn versions_txt_parses_pinned_versions() {
        let txt = "Package: foot Version: 1.21.0-2 Architecture: arm64 \n\
                   Package: weston Version: 14.0.2-1 Architecture: arm64 \n";
        assert_eq!(pinned_version(txt, "weston").as_deref(), Some("14.0.2-1"));
        assert_eq!(pinned_version(txt, "foot").as_deref(), Some("1.21.0-2"));
        assert_eq!(pinned_version(txt, "chromium"), None);
    }
}

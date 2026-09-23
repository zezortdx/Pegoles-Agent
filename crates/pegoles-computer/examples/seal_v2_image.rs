//! Seal an already-provisioned work disk as Pegoles Guest Image v2.
//!
//! BUILD TIME ONLY. Used after qemu-assisted (fully observable)
//! provisioning powers the VM off: hashes the bytes on disk, writes the
//! marker + v2 manifest (graphical facts from $PEGOLES_DEBS_DIR,
//! capabilities input+frame). Never invents hashes.
//!
//! Usage:
//!   PEGOLES_DEBS_DIR=/tmp/pgv2-debs \
//!     cargo run -p pegoles-computer --example seal_v2_image -- /path/to/disk.img

use pegoles_computer::{
    pegoles_data_dir, ComputerImageManager, DerivedManifestInput, GraphicalImageInfo,
    PEGOLES_BASE_IMAGE_ID_V2, PEGOLES_IMAGE_VERSION_V2,
};
use pegoles_guest_proto::{GUEST_PROTOCOL_VERSION, RUNTIME_VERSION};

fn pinned_version(versions_txt: &str, package: &str) -> Option<String> {
    for line in versions_txt.lines() {
        let mut parts = line.split_whitespace();
        if parts.next()? != "Package:" {
            continue;
        }
        if parts.next()? != package {
            continue;
        }
        let rest: Vec<&str> = parts.collect();
        if let Some(i) = rest.iter().position(|t| *t == "Version:") {
            if let Some(v) = rest.get(i + 1) {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn main() {
    let disk = std::env::args()
        .nth(1)
        .expect("usage: seal_v2_image /path/to/disk.img");
    // Truncation guard (Phase 5.1 lesson): a provisioned generic-derived
    // disk is exactly 3221225472 bytes. Sealing anything else records a
    // self-consistent hash of garbage — fail LOUDLY instead.
    // (ENOSPC mid-copy is the classic cause; free space first.)
    const EXPECTED_DISK_BYTES: u64 = 3_221_225_472;
    let len = std::fs::metadata(&disk).expect("stat work disk").len();
    assert_eq!(
        len, EXPECTED_DISK_BYTES,
        "work disk has unexpected size {len} (truncated copy?)"
    );
    let data = pegoles_data_dir();
    let graphical = std::env::var("PEGOLES_DEBS_DIR")
        .ok()
        .and_then(|dir| std::fs::read_to_string(format!("{dir}/VERSIONS.txt")).ok())
        .and_then(|text| {
            Some(GraphicalImageInfo {
                compositor: format!("weston {}", pinned_version(&text, "weston")?),
                terminal: format!("foot {}", pinned_version(&text, "foot")?),
                browser: None,
            })
        });
    if graphical.is_none() {
        eprintln!("WARNING: no VERSIONS.txt graphical facts; sealing headless v0.2");
    }
    // Source hash: read the verified marker of the official generic
    // artifact (computed at download, never invented). Fails closed.
    let marker = data.join("images/debian-13-generic-arm64/base.raw.verified");
    let source_sha = std::fs::read_to_string(&marker)
        .map(|s| s.trim().to_string())
        .expect("generic base.raw.verified marker missing");
    assert!(
        source_sha.chars().all(|c| c.is_ascii_hexdigit()) && source_sha.len() == 128,
        "marker is not a sha512 hex digest"
    );
    let mgr = ComputerImageManager::new(data.join("images"));
    let input = DerivedManifestInput {
        image_id: PEGOLES_BASE_IMAGE_ID_V2.to_string(),
        image_version: PEGOLES_IMAGE_VERSION_V2.to_string(),
        debian_version: "13".into(),
        architecture: "arm64".into(),
        guest_runtime_version: RUNTIME_VERSION.into(),
        guest_protocol_version: GUEST_PROTOCOL_VERSION,
        source_image_sha512: source_sha,
        graphical,
        capabilities: vec!["input".to_string(), "frame".to_string()],
    };
    let manifest = mgr
        .publish_derived(std::path::Path::new(&disk), input)
        .expect("publish derived v2");
    println!(
        "sealed manifest:\n{}",
        serde_json::to_string_pretty(&manifest).unwrap()
    );
    println!("DONE: images/pegoles-base-0.2 sealed (verify marker + manifest above)");
}

//! Build-time provisioner: boots the official generic (cloud-init) image
//! with the NoCloud seed ISO attached, waits for cloud-init to install the
//! guest runtime and power off, then seals the disk as images/pegoles-base-0.1.
//!
//! BUILD TIME ONLY. The normal lifecycle never sets PEGOLES_IMAGE_SPEC or
//! PEGOLES_SEED_ISO.
//!
//! Usage:
//!   PEGOLES_IMAGE_SPEC=generic PEGOLES_SEED_ISO=/path/to/seed.iso \
//!     cargo run -p pegoles-computer --example provision_base_image

use pegoles_computer::{
    default_config, pegoles_data_dir, ComputerBackend, ComputerImageManager, DerivedManifestInput,
    MacOSVirtualizationBackend, GENERIC_DEBIAN_13_ARM64,
};
use pegoles_guest_proto::{GUEST_PROTOCOL_VERSION, RUNTIME_VERSION};
use std::time::{Duration, Instant};

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

    // 2. Boot it once with the seed ISO attached.
    let mut backend = MacOSVirtualizationBackend::new(data.join("images"), data.join("computers"))
        .expect("macos backend");
    let id = backend.create(default_config()).expect("create");
    println!("build computer: {id}");
    backend.start().expect("start");

    // 3. Wait for cloud-init to finish and power the VM off.
    let deadline = Instant::now() + Duration::from_secs(12 * 60);
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

    // 4. Seal the disk as the derived image (hashes computed, not invented).
    let disk = data.join("computers").join(id.to_string()).join("disk.img");
    let mgr = ComputerImageManager::new(data.join("images"));
    let manifest = mgr
        .publish_derived(
            &disk,
            DerivedManifestInput::v0_1(
                "13".into(),
                "arm64".into(),
                RUNTIME_VERSION.into(),
                GUEST_PROTOCOL_VERSION,
                source.sha512.clone(),
            ),
        )
        .expect("publish derived");
    println!(
        "derived manifest:\n{}",
        serde_json::to_string_pretty(&manifest).unwrap()
    );

    // 5. Drop the build computer (helper child dies with the backend).
    std::fs::remove_dir_all(data.join("computers").join(id.to_string())).ok();
    println!("DONE: images/pegoles-base-0.1 is Ready");
}

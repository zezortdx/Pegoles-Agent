//! Install the pinned release guest image through the product installer,
//! from HTTPS or, for release engineering, from a local copy of the
//! published archive. Every byte is still checked against the pins compiled
//! into this build.
//!
//! ```sh
//! # into the real data dir, over HTTPS: exactly the call the app's setup
//! # makes (ComputerImageManager::install_release_image):
//! cargo run --release -p pegoles-computer --example install_image
//! # into a scratch images dir, from the archive the release will publish:
//! cargo run --release -p pegoles-computer --example install_image -- \
//!   --images-dir /tmp/pegoles-images --from-file target/guest-image/pegoles-base-0.3-arm64.raw.gz
//! # another host's pinned image (e.g. the Windows x64 one from a Mac),
//! # over HTTPS, into a scratch dir:
//! cargo run --release -p pegoles-computer --example install_image -- \
//!   --image pegoles-base-x64-0.1 --images-dir /tmp/pegoles-x64
//! # only re-hash an installed image against its pin (exit 1 on mismatch):
//! cargo run --release -p pegoles-computer --example install_image -- --verify-only
//! ```

use std::path::PathBuf;
use std::time::Instant;

use pegoles_computer::image_release::{
    self, ArchiveSource, HttpsSource, InstallStage, LocalArchiveSource, ReleaseImage,
};
use pegoles_computer::{active_image_id, pegoles_data_dir, ComputerImageManager};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let images_dir = value("--images-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| pegoles_data_dir().join("images"));
    // `--image <id>` picks another pinned catalog entry (never a path or an
    // unpinned id): release engineering checks every host's image anywhere.
    let chosen = value("--image");
    let id = chosen.clone().unwrap_or_else(active_image_id);
    let pin: &ReleaseImage = image_release::release_image(&id).expect("pinned release image");

    if args.iter().any(|a| a == "--verify-only") {
        let dir = images_dir.join(&pin.id);
        let started = Instant::now();
        match image_release::verify_installed(&dir, pin) {
            Ok(()) => println!(
                "verified {} at {} in {:.1}s",
                pin.id,
                dir.display(),
                started.elapsed().as_secs_f64()
            ),
            Err(e) => {
                eprintln!("verification failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let started = Instant::now();
    let mut last = (InstallStage::Downloading, 0u64);
    let mut report = |stage: InstallStage, done: u64, total: u64| {
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        if stage != last.0 || pct >= last.1 + 10 {
            println!("{:>10} {pct:>3}% ({done}/{total})", stage.as_str());
            last = (stage, pct);
        }
    };
    let from_file = value("--from-file").map(PathBuf::from);
    let result = if from_file.is_none() && chosen.is_none() {
        // The product path: this build's own image, over HTTPS.
        println!(
            "setup path: {} from {}",
            pin.id,
            pin.archive.urls.join(", ")
        );
        ComputerImageManager::new(images_dir.clone()).install_release_image(&mut report, &|| false)
    } else {
        let mut image = pin.clone();
        let source: Box<dyn ArchiveSource> = match from_file {
            Some(file) => {
                // The local copy stands in for the URL; the archive and disk
                // pins are unchanged.
                image.archive.urls = vec!["https://local.file/archive".into()];
                Box::new(LocalArchiveSource(file))
            }
            None => Box::new(HttpsSource::default()),
        };
        image_release::install(&images_dir, &image, source.as_ref(), &mut report, &|| false)
    };
    match result {
        Ok(dir) => {
            let t = started.elapsed();
            image_release::verify_installed(&dir, pin).expect("post-install verification");
            println!(
                "installed and verified {} ({}) at {} in {:.1}s (re-verify {:.1}s)",
                pin.id,
                pin.disk.file_name,
                dir.display(),
                t.as_secs_f64(),
                started.elapsed().as_secs_f64() - t.as_secs_f64()
            );
        }
        Err(e) => {
            eprintln!("install failed: {e}");
            std::process::exit(1);
        }
    }
}

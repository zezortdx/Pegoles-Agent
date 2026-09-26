//! Install the pinned release guest image through the product installer
//! (`image_release::install`), from HTTPS or, for release engineering, from
//! a local copy of the published archive. Every byte is still checked
//! against the pins compiled into this build.
//!
//! ```sh
//! # into a scratch images dir, from the archive the release will publish:
//! cargo run --release -p pegoles-computer --example install_image -- \
//!   --images-dir /tmp/pegoles-images --from-file target/guest-image/pegoles-base-0.3-arm64.raw.gz
//! # into the real data dir, over HTTPS (needs a published catalog URL):
//! cargo run --release -p pegoles-computer --example install_image
//! ```

use std::path::PathBuf;
use std::time::Instant;

use pegoles_computer::image_release::{
    self, ArchiveSource, HttpsSource, InstallStage, LocalArchiveSource, ReleaseImage,
};
use pegoles_computer::{active_image_id, pegoles_data_dir};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let images_dir = arg("--images-dir").unwrap_or_else(|| pegoles_data_dir().join("images"));
    let id = active_image_id();
    let pin: &ReleaseImage = image_release::release_image(&id).expect("pinned release image");
    let mut image = pin.clone();
    let source: Box<dyn ArchiveSource> = match arg("--from-file") {
        Some(file) => {
            // The local copy stands in for the (possibly unpublished) URL;
            // the archive and disk pins are unchanged.
            image.archive.urls = vec!["https://local.file/archive".into()];
            Box::new(LocalArchiveSource(file))
        }
        None => Box::new(HttpsSource::default()),
    };
    let started = Instant::now();
    let mut last = (InstallStage::Downloading, 0u64);
    let result = image_release::install(
        &images_dir,
        &image,
        source.as_ref(),
        &mut |stage, done, total| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if stage != last.0 || pct >= last.1 + 10 {
                println!("{:>10} {pct:>3}% ({done}/{total})", stage.as_str());
                last = (stage, pct);
            }
        },
        &|| false,
    );
    match result {
        Ok(dir) => {
            let t = started.elapsed();
            image_release::verify_installed(&dir, pin).expect("post-install verification");
            println!(
                "installed and verified {} at {} in {:.1}s (re-verify {:.1}s)",
                pin.id,
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

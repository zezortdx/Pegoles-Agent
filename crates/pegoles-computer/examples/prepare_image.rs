//! Manual image preparation: download + verify + extract the official
//! Debian base image with progress on stderr.
//!
//! Usage: `cargo run -p pegoles-computer --example prepare_image`
//! Respects PEGOLES_DATA_DIR (default: ~/Library/Application Support/Pegoles).

use pegoles_computer::{pegoles_data_dir, ComputerImageManager};

fn main() {
    let data = pegoles_data_dir();
    println!("data dir: {}", data.display());
    let mgr = ComputerImageManager::new(data.join("images"));
    println!("status before: {:?}", mgr.status());
    match mgr.prepare(&mut |stage, downloaded, total| {
        eprintln!("stage={stage:?} downloaded={downloaded} total={total}");
    }) {
        Ok(image) => {
            println!("READY: {} at {}", image.spec.id, image.base_raw.display());
            println!("sha512: {}", image.sha512);
        }
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}

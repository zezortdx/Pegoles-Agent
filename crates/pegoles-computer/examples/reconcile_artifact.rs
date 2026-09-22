//! Reconcile an already-sealed artifact file into the derived manifest.
//!
//! Usage:
//!   cargo run -p pegoles-computer --example reconcile_artifact --
//!     <file-name> <raw|vhdx> <arch> <runtime-version> <source-sha512>
//!
//! Hashes bytes in place, verifies against the on-disk marker when
//! present, merges the record. Used to repair/extend manifests.

use pegoles_computer::{pegoles_data_dir, ComputerImageManager, DiskFormat};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!("usage: reconcile_artifact <file> <raw|vhdx> <arch> <runtime> <source-sha>");
        std::process::exit(2);
    }
    let format = match args[2].as_str() {
        "raw" => DiskFormat::Raw,
        "vhdx" => DiskFormat::Vhdx,
        other => {
            eprintln!("unknown format: {other}");
            std::process::exit(2);
        }
    };
    let mgr = ComputerImageManager::new(pegoles_data_dir().join("images"));
    let manifest = mgr
        .reconcile_artifact(
            &args[1],
            format,
            args[3].clone(),
            args[4].clone(),
            args[5].clone(),
        )
        .expect("reconcile");
    println!("reconciled {} artifacts:", manifest.artifacts.len());
    for a in &manifest.artifacts {
        println!("- {} {:?} {} bytes", a.file_name, a.disk_format, a.bytes);
    }
}

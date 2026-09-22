//! Build-time Windows artifact: official generic amd64 -> verified RAW ->
//! qemu-img convert to dynamic VHDX -> kernel gate -> publish into the
//! shared derived image dir as `disk.vhdx` (own hash, own manifest entry).
//!
//! BUILD TIME ONLY (macOS/Linux build machine). The Windows runtime never
//! sees qemu-img: it boots the finished VHDX. The artifact is recorded as
//! UNPROVISIONED (no guest runtime inside yet): first boot on Windows
//! hardware provisions it via the seed-ISO flow, exactly like macOS.
//!
//! Usage:
//!   cargo run -p pegoles-computer --example build_windows_artifact

use pegoles_computer::{
    pegoles_data_dir, ComputerImageManager, DerivedManifestInput, GENERIC_DEBIAN_13_AMD64,
};
use pegoles_guest_proto::GUEST_PROTOCOL_VERSION;
use std::path::PathBuf;

fn main() {
    let data = pegoles_data_dir();
    println!("data dir: {}", data.display());

    // 1. Official generic amd64, verified like everything else.
    let generic = ComputerImageManager::with_spec(data.join("images"), GENERIC_DEBIAN_13_AMD64);
    println!("generic-amd64 status: {:?}", generic.status());
    let source = generic
        .prepare(&mut |stage, downloaded, total| {
            eprintln!("generic-amd64: {stage:?} {downloaded}/{total}");
        })
        .expect("prepare generic amd64 image");
    println!("generic ready: {}", source.base_raw.display());

    // 2. Convert to dynamic VHDX (build tooling only).
    let work_vhdx = data
        .join("images")
        .join("debian-13-generic-amd64")
        .join("disk.vhdx");
    let status = std::process::Command::new("qemu-img")
        .args([
            "convert",
            "-f",
            "raw",
            "-O",
            "vhdx",
            "-o",
            "subformat=dynamic",
        ])
        .arg(&source.base_raw)
        .arg(&work_vhdx)
        .status()
        .expect("run qemu-img (brew install qemu on the build machine)");
    assert!(status.success(), "qemu-img convert failed");
    let vhdx_bytes = std::fs::metadata(&work_vhdx).expect("vhdx").len();
    println!("vhdx: {} ({vhdx_bytes} bytes sparse)", work_vhdx.display());

    // 3. Kernel gate on the SOURCE raw (same bytes that became the VHDX).
    check_kernel(&source.base_raw);

    // 4. Publish as the Windows artifact of the logical image.
    let mgr = ComputerImageManager::new(data.join("images"));
    let manifest = mgr
        .publish_derived_as(
            &work_vhdx,
            DerivedManifestInput {
                debian_version: "13".into(),
                architecture: "amd64".into(),
                guest_runtime_version: "pending-first-boot-provisioning".into(),
                guest_protocol_version: GUEST_PROTOCOL_VERSION,
                source_image_sha512: source.sha512.clone(),
            },
            "disk.vhdx",
            pegoles_computer::DiskFormat::Vhdx,
        )
        .expect("publish vhdx");
    println!(
        "VHDX artifact:\n{}",
        serde_json::to_string_pretty(
            &manifest
                .artifacts
                .iter()
                .find(|a| a.file_name == "disk.vhdx")
                .expect("vhdx record")
        )
        .unwrap()
    );
    println!("DONE: disk.vhdx published (UNPROVISIONED - see WINDOWS_IMAGE.md)");
}

/// Read-only kernel config check via debugfs (no mounts, no writes).
/// Fails closed: vsock transports must be provable in the source kernel.
/// The raw starts with a GPT; debugfs needs the ext4 partition bytes, so
/// extract the root partition to a temp file first (pure-Rust GPT parse).
fn check_kernel(raw: &PathBuf) {
    use std::io::{Read, Seek, SeekFrom};
    let debugfs = "/opt/homebrew/opt/e2fsprogs/sbin/debugfs";
    let (offset, len) = pegoles_computer::gpt_root_partition(raw).expect("GPT parse");
    println!("root partition at byte {offset} ({len} bytes)");
    let part = raw.with_extension("rootpart");
    {
        let mut src = std::fs::File::open(raw).expect("open raw");
        let mut dst = std::fs::File::create(&part).expect("create part");
        src.seek(SeekFrom::Start(offset)).expect("seek");
        let mut left = len;
        let mut buf = [0u8; 1024 * 1024];
        while left > 0 {
            let want = left.min(buf.len() as u64) as usize;
            let n = src.read(&mut buf[..want]).expect("read");
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut dst, &buf[..n]).expect("write");
            left -= n as u64;
        }
    }
    // List /boot to find the config file (exact kernel version varies).
    let ls = std::process::Command::new(debugfs)
        .args(["-R", "ls /boot"])
        .arg(&part)
        .output()
        .expect("run debugfs");
    let listing = String::from_utf8_lossy(&ls.stdout);
    let config = listing
        .lines()
        .filter_map(|l| {
            let name = l.split_whitespace().last()?;
            name.strip_prefix("config-").map(|_| name.to_string())
        })
        .next()
        .expect("no /boot/config-* in image");
    println!("kernel config: {config}");
    let cat = std::process::Command::new(debugfs)
        .args(["-R", &format!("cat /boot/{config}")])
        .arg(&part)
        .output()
        .expect("run debugfs");
    assert!(cat.status.success(), "debugfs cat failed");
    let text = String::from_utf8_lossy(&cat.stdout).into_owned();
    let _ = std::fs::remove_file(&part);
    let caps = pegoles_computer::check_kernel_config(&text);
    println!(
        "kernel caps: vsock={:?} virtio={:?} hyperv={:?} universal={}",
        caps.vsock,
        caps.virtio_vsock,
        caps.hyperv_vsock,
        caps.universal_ready()
    );
    assert!(
        caps.universal_ready(),
        "amd64 kernel lacks virtio+hyperv vsock; refusing artifact"
    );
}

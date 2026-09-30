// An app ACL manifest makes Tauri check every app command against the
// capabilities, local origin included; without one, any script in the
// webview could call every registered command.
macro_rules! ipc_commands {
    (release: [$($release:ident),* $(,)?], debug: [$($debug:ident),* $(,)?] $(,)?) => {
        /// Every command gets an `allow-<command>` permission; the
        /// capabilities decide which builds grant it.
        const COMMANDS: &[&str] = &[$(stringify!($release),)* $(stringify!($debug),)*];
    };
}
include!("src/ipc_commands.rs");

/// The commit this build came from, for the diagnostic report: CI's
/// `GITHUB_SHA`, else `git rev-parse` in a checkout, else nothing.
fn build_commit() -> Option<String> {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        if sha.len() >= 12 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(sha[..12].to_string());
        }
    }
    let git =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").ok()?).join("../../../.git");
    if git.is_dir() {
        println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git.join("refs").display());
    }
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && sha.len() == 12 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(sha)
}

fn main() {
    if let Some(commit) = build_commit() {
        println!("cargo:rustc-env=PEGOLES_BUILD_COMMIT={commit}");
    }
    println!("cargo:rerun-if-changed=src/ipc_commands.rs");
    println!("cargo:rerun-if-changed=windows/app.manifest");
    let mut attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    // Windows: the application manifest (Common Controls v6) goes into
    // every binary of this package through the linker, not only the app's
    // resources; a test binary without it cannot even load
    // (STATUS_ENTRYPOINT_NOT_FOUND for TaskDialogIndirect).
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest =
            std::path::Path::new(&target("CARGO_MANIFEST_DIR")).join("windows/app.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        println!("{error:#}");
        std::process::exit(1);
    }
}

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

fn main() {
    println!("cargo:rerun-if-changed=src/ipc_commands.rs");
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(error) = tauri_build::try_build(attributes) {
        println!("{error:#}");
        std::process::exit(1);
    }
}

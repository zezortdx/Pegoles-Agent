//! macOS native display glue (Phase 4). CONTRACT STUB — implemented by the
//! native-embed stream: installs the in-process VM host transport factory
//! (`pegoles_computer::native_backend::install_host_transport_factory`)
//! and a `MacVirtualMachineDisplay` bound to the main window
//! (`AppState.registry.install_display_backend`).

pub fn install(_app: &tauri::App) -> Result<(), String> {
    Ok(())
}

//! Tauri backend: owns Pegoles Core behind a shared Mutex and exposes it
//! via commands. The frontend never holds core state; every button calls
//! into Rust. Commands that touch Core run off the main thread (see
//! `commands.rs`): the in-process VM needs the main queue free.

pub mod agent;
pub mod commands;
pub mod consent;
pub mod local;
#[cfg(target_os = "macos")]
mod native_display;
pub mod nav_guard;
pub mod onboarding;
pub mod state;
pub mod webview_egress;

#[cfg(test)]
mod ipc_acl;

use agent::AgentSupervisor;
use commands::{SharedState, StatusCache};
use state::AppState;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

/// Background cadence for Core maintenance (guest heartbeat, handshake,
/// display, lifecycle) independent of whether the UI is polling.
const PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Every command the webview may call, from src/ipc_commands.rs (also
/// read by build.rs for the ACL). Debug builds add the Design Lab and
/// diagnostics commands and the capability that grants them.
macro_rules! ipc_commands {
    (release: [$($release:ident),* $(,)?], debug: [$($debug:ident),* $(,)?] $(,)?) => {
        /// Commands the release webview may call (capabilities/default.json).
        pub const RELEASE_COMMANDS: &[&str] = &[$(stringify!($release)),*];
        /// Debug-build extras (debug-capabilities/design-lab.json).
        pub const DEBUG_COMMANDS: &[&str] = &[$(stringify!($debug)),*];

        #[cfg(debug_assertions)]
        fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$(commands::$release,)* $(commands::$debug,)*]
        }

        #[cfg(not(debug_assertions))]
        fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$(commands::$release,)*]
        }
    };
}
include!("ipc_commands.rs");

/// Grants the debug-only commands to the main window. Debug builds only:
/// release builds neither register nor grant them.
#[cfg(any(debug_assertions, test))]
fn grant_debug_commands<R: tauri::Runtime>(app: &impl Manager<R>) -> tauri::Result<()> {
    app.add_capability(include_str!("../debug-capabilities/design-lab.json"))
}

/// The app's config, assets and ACL (capabilities/, the build.rs manifest).
fn context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub fn run() {
    let app_state: SharedState = Arc::new(Mutex::new(AppState::new()));
    let status_cache = StatusCache::default();
    let supervisor = AgentSupervisor::default();

    let builder = tauri::Builder::default()
        .manage(app_state)
        .manage(status_cache)
        .manage(supervisor)
        .manage(local::LocalModels::default())
        .manage(commands::ScriptCancel::default())
        .invoke_handler(invoke_handler());

    let app = builder
        .setup(|app| {
            // Switching to a cloud planner and storing its key are
            // confirmed in native alerts the webview can't answer.
            app.manage(consent::ConsentGate(Arc::new(consent::NativeConsent::new(
                app.handle().clone(),
            ))));
            #[cfg(debug_assertions)]
            grant_debug_commands(app)?;
            // Pegoles is open again: the one-time "resume after the
            // setup restart" entry has done its job (no-op elsewhere).
            pegoles_computer::windows::clear_resume_after_restart();
            // The only window, with navigation kept on the app origin and
            // new windows refused (see nav_guard.rs).
            let boot = {
                let shared: SharedState = app.state::<SharedState>().inner().clone();
                let local = app.state::<local::LocalModels>().inner().clone();
                onboarding::boot_script(&pegoles_computer::pegoles_data_dir(), || {
                    commands::already_set_up(&shared, &local)
                })
            };
            nav_guard::build_main_window(app, boot)?;
            // Platform display adapter (macOS: in-process VM host + native
            // framebuffer view). Failure is recorded, never fatal, never
            // replaced by a fake display.
            #[cfg(target_os = "macos")]
            if let Err(e) = native_display::install(app) {
                let shared: SharedState = app.state::<SharedState>().inner().clone();
                commands::lock_state(&shared).display_error = Some(e);
            }
            // Bridge EventBus -> frontend `pegoles://event` emissions.
            // History (`list_events`) drains its own subscriber in AppState.
            let handle = app.handle().clone();
            let shared: SharedState = app.state::<SharedState>().inner().clone();
            let mut rx = commands::lock_state(&shared).subscribe();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("pegoles://event", &event);
                        }
                        // A burst larger than the bus: skip ahead, keep
                        // streaming (history still has what it drained).
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            // Core maintenance does not depend on the UI polling (a hidden
            // window must not let the guest session time out). Skips a
            // tick when a command holds the state.
            let cache = app.state::<StatusCache>().inner().clone();
            let agent = app.state::<AgentSupervisor>().inner().clone();
            let local = app.state::<local::LocalModels>().inner().clone();
            let pump_state = shared.clone();
            let pump_local = local.clone();
            std::thread::Builder::new()
                .name("pegoles-pump".into())
                .spawn(move || loop {
                    std::thread::sleep(PUMP_INTERVAL);
                    commands::pump_if_idle(&pump_state, &cache, &agent);
                    pump_local.idle_check();
                })?;
            // Planner readiness for the status payload (Keychain read and
            // model checks off the main thread), then verify the local
            // model's bytes so the first task does not wait for hashing.
            std::thread::spawn(move || {
                commands::refresh_readiness(&shared, &local);
                let settings = agent::load_settings();
                if settings.provider == agent::Provider::Local {
                    local.prewarm_verify(&settings.local_model);
                }
            });
            Ok(())
        })
        .build(context())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            // Stop any agent run. The VM helper stops its VMs and exits on
            // its own when this process closes its stdin (see
            // native/macos/pegoles-vm-host/Sources/Host.swift).
            app.state::<AgentSupervisor>().cancel(None);
            app.state::<commands::ScriptCancel>().trip();
            app.state::<local::LocalModels>().shutdown();
        }
    });
}

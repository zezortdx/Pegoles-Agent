//! Tauri backend: owns Pegoles Core behind a shared Mutex and exposes it
//! via commands. The frontend never holds core state; every button calls
//! into Rust. Commands that touch Core run off the main thread (see
//! `commands.rs`): the in-process VM needs the main queue free.

pub mod agent;
pub mod commands;
#[cfg(target_os = "macos")]
mod native_display;
pub mod state;

use agent::AgentSupervisor;
use commands::{SharedState, StatusCache};
use state::AppState;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

/// Background cadence for Core maintenance (guest heartbeat, handshake,
/// display, lifecycle) independent of whether the UI is polling.
const PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Every command the webview may call. Debug builds add the Design Lab
/// commands that drive raw actions and scripts.
macro_rules! app_handlers {
    ($($extra:path,)*) => {
        tauri::generate_handler![
            commands::get_status,
            commands::pump,
            commands::create_computer,
            commands::start_computer,
            commands::pause_computer,
            commands::resume_computer,
            commands::stop_computer,
            commands::list_events,
            commands::get_image_status,
            commands::prepare_image,
            commands::read_boot_log,
            commands::guest_info,
            commands::guest_ping,
            commands::get_host_capabilities,
            commands::accessibility_display,
            commands::suggested_config,
            commands::suggested_effects,
            commands::display_set_geometry,
            commands::display_detach,
            commands::take_control,
            commands::return_control,
            commands::create_task,
            commands::list_tasks,
            commands::run_task,
            commands::cancel_task,
            commands::get_model_settings,
            commands::set_api_key,
            commands::clear_api_key,
            commands::set_model_settings,
            commands::cancel_agent_input,
            commands::capture_screen,
            commands::input_status,
            commands::input_audit,
            $($extra,)*
        ]
    };
}

pub fn run() {
    let app_state: SharedState = Arc::new(Mutex::new(AppState::new()));
    let status_cache = StatusCache::default();
    let supervisor = AgentSupervisor::default();

    let builder = tauri::Builder::default()
        .manage(app_state)
        .manage(status_cache)
        .manage(supervisor)
        .manage(commands::ScriptCancel::default());
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(app_handlers!(
        commands::execute_action,
        commands::run_input_script,
        commands::demo_script_steps,
    ));
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(app_handlers!());

    let app = builder
        .setup(|app| {
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
            let pump_state = shared.clone();
            std::thread::Builder::new()
                .name("pegoles-pump".into())
                .spawn(move || loop {
                    std::thread::sleep(PUMP_INTERVAL);
                    commands::pump_if_idle(&pump_state, &cache, &agent);
                })?;
            // Model availability for the status payload (Keychain read off
            // the main thread).
            std::thread::spawn(move || {
                let configured = agent::load_api_key().is_some();
                commands::lock_state(&shared).model_configured = configured;
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            // Stop any agent run. The VM helper stops its VMs and exits on
            // its own when this process closes its stdin (see
            // native/macos/pegoles-vm-host/Sources/Host.swift).
            app.state::<AgentSupervisor>().cancel(None);
            app.state::<commands::ScriptCancel>().trip();
        }
    });
}

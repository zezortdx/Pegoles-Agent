//! Tauri backend: owns Pegoles Core behind a shared Mutex and exposes it
//! via commands. The frontend never holds core state; every button calls
//! into Rust. Commands that touch Core run off the main thread (see
//! `commands.rs`): the in-process VM needs the main queue free.

pub mod commands;
#[cfg(target_os = "macos")]
mod native_display;
pub mod state;

use commands::SharedState;
use state::AppState;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

pub fn run() {
    let app_state: SharedState = Arc::new(Mutex::new(AppState::new()));

    tauri::Builder::default()
        .manage(app_state)
        .manage(commands::ScriptCancel::default())
        .invoke_handler(tauri::generate_handler![
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
            commands::execute_action,
            commands::cancel_agent_input,
            commands::capture_screen,
            commands::run_input_script,
            commands::demo_script_steps,
            commands::input_status,
            commands::input_audit,
        ])
        .setup(|app| {
            // Platform display adapter (macOS: in-process VM host + native
            // framebuffer view). Failure is recorded, never fatal, never
            // replaced by a fake display.
            #[cfg(target_os = "macos")]
            if let Err(e) = native_display::install(app) {
                let shared: SharedState = app.state::<SharedState>().inner().clone();
                let mut guard = shared
                    .lock()
                    .map_err(|_| "app state lock poisoned during setup")?;
                guard.display_error = Some(e);
            }
            // Bridge EventBus -> frontend `pegoles://event` emissions.
            // History (`list_events`) drains its own subscriber in AppState.
            let handle = app.handle().clone();
            let mut rx = {
                let state: tauri::State<'_, SharedState> = handle.state();
                let guard = state
                    .lock()
                    .map_err(|_| "app state lock poisoned during setup")?;
                guard.subscribe()
            };
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

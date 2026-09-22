//! Application entry point (Tauri 2 pattern: lib.rs owns setup).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pegoles_desktop::run();
}

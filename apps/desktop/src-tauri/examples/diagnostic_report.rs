//! Write the diagnostic report this machine would produce ("Save a report
//! for help") into a folder and check it holds no home-folder path. Used
//! by CI on Windows and macOS to see real facts pass through the report.
//!
//! ```sh
//! cargo run -p pegoles-desktop --example diagnostic_report -- --out /tmp
//! ```

use std::sync::{Arc, Mutex};

use pegoles_desktop::agent::AgentSupervisor;
use pegoles_desktop::commands::write_diagnostic_report;
use pegoles_desktop::local::LocalModels;
use pegoles_desktop::state::AppState;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let shared = Arc::new(Mutex::new(AppState::new()));
    let report = write_diagnostic_report(
        &shared,
        &AgentSupervisor::default(),
        &LocalModels::default(),
        Some(&out),
    )
    .unwrap_or_else(|e| {
        eprintln!("report failed: {e}");
        std::process::exit(1);
    });
    println!("{}", report.text);
    println!("saved {} in {}", report.file_name, out.display());
    let home = pegoles_desktop::diagnostics::home_dir();
    if let Some(home) = home.as_ref().map(|h| h.to_string_lossy().into_owned()) {
        if report.text.contains(&home) {
            eprintln!("the report contains the home folder path");
            std::process::exit(1);
        }
    }
}

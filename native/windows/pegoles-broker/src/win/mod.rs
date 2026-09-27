//! Windows implementation of the broker's verbs.

mod client;
mod files;
mod hcs;
mod server;
mod service;
mod setup;
mod util;

pub fn main(args: &[String]) -> i32 {
    match args.get(1).map(String::as_str) {
        Some("service") => service::run(),
        Some("run-foreground") => match server::serve(&server::StopFlag::default()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pegoles-broker: {e}");
                1
            }
        },
        Some("install-service") => report(setup::install_service()),
        Some("uninstall-service") => report(setup::uninstall_service()),
        Some("enable-virtualization") => setup::enable_virtualization(),
        _ => {
            eprintln!("usage: pegoles-broker service | run-foreground | install-service | uninstall-service | enable-virtualization | --version");
            2
        }
    }
}

fn report(result: Result<(), String>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pegoles-broker: {e}");
            1
        }
    }
}

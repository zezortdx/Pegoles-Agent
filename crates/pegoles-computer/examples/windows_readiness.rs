//! What onboarding's system check sees on this PC: the Windows edition
//! and version, whether virtualization is ready (or can be turned on, or
//! is off in the firmware, or waits for a restart), and whether Pegoles'
//! broker service is installed. Read-only: nothing is changed.
//!
//! ```sh
//! cargo run -p pegoles-computer --example windows_readiness
//! ```

fn main() {
    let r = pegoles_computer::windows::readiness();
    println!("os: {} (supported: {})", r.os_name, r.os_supported);
    println!("virtualization: {:?} (Pegoles can fix it: {})", r.state, r.fixable);
    println!("broker installed: {}", r.broker_installed);
    println!("technical: {}", r.technical);
}

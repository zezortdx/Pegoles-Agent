//! How the runtime reaches the host's control plane.
//!
//! The Guest Protocol above never branches on hypervisors; only the
//! connection's direction differs, and it is decided here:
//!
//! - **Dial** (Apple Virtualization, virtio-vsock): connect to the host,
//!   CID 2 (`VMADDR_CID_HOST`), port 4050, from a reserved source port.
//!   The host checks that port.
//! - **Listen** (Hyper-V, hv_sock): listen on reserved port 850 and let
//!   the host connect. Windows cannot reliably see a guest's source port,
//!   so owning the privileged listener is what proves "this is the
//!   runtime".
//!
//! The mode comes from the command line (`--listen`, set by the image's
//! unit) or the kernel command line (`pegoles.transport=listen`, set by
//! the host that boots the kernel). Default: dial.

use pegoles_guest_proto::{PEGOLES_GUEST_LISTEN_PORT, PEGOLES_VSOCK_PORT};

/// Host endpoint for the control plane (dial mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestEndpoint {
    /// vsock CID of the host (2 == VMADDR_CID_HOST on every transport).
    pub host_cid: u32,
    pub port: u32,
}

/// Where the connection comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Connect to the host (virtio-vsock hosts).
    Dial(GuestEndpoint),
    /// Accept the host's connection on this privileged port (Hyper-V).
    Listen { port: u32 },
}

/// The host endpoint for dial mode: CID 2 + port 4050.
pub fn resolve_host_endpoint() -> GuestEndpoint {
    GuestEndpoint {
        host_cid: 2,
        port: PEGOLES_VSOCK_PORT,
    }
}

/// Pure: the mode from our arguments and the kernel command line.
pub fn resolve_mode<S: AsRef<str>>(args: &[S], kernel_cmdline: &str) -> Mode {
    let listen_arg = args.iter().any(|a| a.as_ref() == "--listen");
    let listen_kernel = kernel_cmdline
        .split_ascii_whitespace()
        .any(|word| word == "pegoles.transport=listen");
    if listen_arg || listen_kernel {
        Mode::Listen {
            port: PEGOLES_GUEST_LISTEN_PORT,
        }
    } else {
        Mode::Dial(resolve_host_endpoint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dials_the_host_by_default() {
        let none: [&str; 0] = [];
        assert_eq!(
            resolve_mode(&none, "root=/dev/vda2 quiet"),
            Mode::Dial(GuestEndpoint {
                host_cid: 2,
                port: 4050
            })
        );
    }

    #[test]
    fn listens_when_the_unit_or_the_host_says_so() {
        assert_eq!(
            resolve_mode(&["pegoles-guest-runtime", "--listen"], ""),
            Mode::Listen { port: 850 }
        );
        assert_eq!(
            resolve_mode(
                &["pegoles-guest-runtime"],
                "root=/dev/sda rw pegoles.transport=listen console=ttyS0"
            ),
            Mode::Listen { port: 850 }
        );
        // Only the exact word counts.
        let none: [&str; 0] = [];
        assert!(matches!(
            resolve_mode(&none, "xpegoles.transport=listen"),
            Mode::Dial(_)
        ));
    }

    #[test]
    fn the_listen_port_is_privileged_and_outside_the_source_range() {
        use pegoles_guest_proto::{GUEST_SOURCE_PORT_MAX, GUEST_SOURCE_PORT_MIN};
        const { assert!(PEGOLES_GUEST_LISTEN_PORT <= 1023) };
        assert!(
            !(GUEST_SOURCE_PORT_MIN..=GUEST_SOURCE_PORT_MAX).contains(&PEGOLES_GUEST_LISTEN_PORT)
        );
    }
}

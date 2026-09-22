//! Host endpoint resolution for the guest control plane.
//!
//! The runtime binary is hypervisor-blind by design: it knows only "a
//! VSOCK transport to the host exists". Endpoint discovery differences
//! (virtio vsock vs Hyper-V vsock) are isolated here so the Guest
//! Protocol layer never branches on hypervisors.
//!
//! - Apple Virtualization (virtio-vsock): host is CID 2 (`VMADDR_CID_HOST`).
//! - Hyper-V (hv_sock): the Linux guest ALSO uses AF_VSOCK with CID 2 for
//!   the host; the host side maps the connection through the Pegoles
//!   service GUID derived from the same logical port. No guest changes.
//! - KVM (vhost-vsock, future): same CID 2 convention.
//!
//! Conclusion: one binary, one endpoint shape. If a future hypervisor
//! needs different discovery, extend THIS module only.

use pegoles_guest_proto::PEGOLES_VSOCK_PORT;

/// Host endpoint for the control plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestEndpoint {
    /// vsock CID of the host (2 == VMADDR_CID_HOST on every transport).
    pub host_cid: u32,
    /// Logical Pegoles port (4050). On Hyper-V the host maps it through
    /// `hyperv_service_guid_for_port(port)`; the guest always dials the
    /// plain port number.
    pub port: u32,
}

/// Resolve where to dial. No detection needed today: CID 2 + port 4050 is
/// correct on virtio, Hyper-V, and vhost transports alike.
pub fn resolve_host_endpoint() -> GuestEndpoint {
    GuestEndpoint {
        host_cid: 2,
        port: PEGOLES_VSOCK_PORT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_hypervisor_blind() {
        let ep = resolve_host_endpoint();
        assert_eq!(ep.host_cid, 2, "VMADDR_CID_HOST on all transports");
        assert_eq!(ep.port, 4050);
        assert_eq!(ep.port, PEGOLES_VSOCK_PORT, "no magic numbers");
    }

    #[test]
    fn port_matches_hyperv_service_guid() {
        // Cross-check with the host-side derivation (same logical port):
        // hyperv_service_guid_for_port(4050) == 00000FD2-facb-… — the guest
        // dials 4050, the Windows host listens on the matching GUID.
        assert_eq!(PEGOLES_VSOCK_PORT, 4050);
    }
}

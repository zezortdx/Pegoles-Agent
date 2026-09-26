//! The guest runtime's systemd unit is part of the channel's peer
//! authentication: only a process holding CAP_NET_BIND_SERVICE can dial
//! from a source port <= `GUEST_SOURCE_PORT_MAX`, and the host accepts no
//! other peer. The image installs the seed unit; the copy next to the
//! runtime sources must never drift from it (it once dropped every
//! capability and restarted only on failure).

use std::path::PathBuf;

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn runtime_unit_matches_the_image_unit() {
    let image = repo_file("scripts/build-guest-image/seed/units/pegoles-guest-runtime.service");
    let runtime = repo_file("guest/runtime/pegoles-guest-runtime.service");
    assert_eq!(
        runtime, image,
        "guest/runtime/pegoles-guest-runtime.service drifted from the image unit"
    );
}

#[test]
fn runtime_unit_grants_exactly_the_reserved_port_capability() {
    const { assert!(pegoles_guest_proto::GUEST_SOURCE_PORT_MAX <= 1023) };
    let unit = repo_file("scripts/build-guest-image/seed/units/pegoles-guest-runtime.service");
    let lines: Vec<&str> = unit.lines().map(str::trim).collect();
    for required in [
        "AmbientCapabilities=CAP_NET_BIND_SERVICE",
        "CapabilityBoundingSet=CAP_NET_BIND_SERVICE",
        "NoNewPrivileges=true",
        "Restart=always",
        "StartLimitIntervalSec=0",
    ] {
        assert!(lines.contains(&required), "missing {required}");
    }
}

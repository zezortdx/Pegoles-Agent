//! IP classification: only global unicast addresses may be connected to.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// `(network, prefix length, name)` for every IPv4 range that is not
/// global unicast.
const V4_BLOCKED: &[(u32, u8, &str)] = &[
    (0x0000_0000, 8, "0.0.0.0/8 this network"),
    (0x0A00_0000, 8, "10.0.0.0/8 private"),
    (0x6440_0000, 10, "100.64.0.0/10 carrier-grade NAT"),
    (0x7F00_0000, 8, "127.0.0.0/8 loopback"),
    (0xA9FE_0000, 16, "169.254.0.0/16 link-local"),
    (0xAC10_0000, 12, "172.16.0.0/12 private"),
    (0xC000_0000, 24, "192.0.0.0/24 IETF protocol assignments"),
    (0xC000_0200, 24, "192.0.2.0/24 documentation"),
    (0xC058_6300, 24, "192.88.99.0/24 6to4 relay"),
    (0xC0A8_0000, 16, "192.168.0.0/16 private"),
    (0xC612_0000, 15, "198.18.0.0/15 benchmarking"),
    (0xC633_6400, 24, "198.51.100.0/24 documentation"),
    (0xCB00_7100, 24, "203.0.113.0/24 documentation"),
    (0xE000_0000, 4, "224.0.0.0/4 multicast"),
    (0xF000_0000, 4, "240.0.0.0/4 reserved and broadcast"),
];

/// IPv6 ranges inside 2000::/3 that are not global unicast.
const V6_BLOCKED: &[(u128, u8, &str)] = &[
    (
        0x2001_0000u128 << 96,
        23,
        "2001::/23 IETF protocol assignments",
    ),
    (0x2001_0db8u128 << 96, 32, "2001:db8::/32 documentation"),
    (0x2002u128 << 112, 16, "2002::/16 6to4"),
    (0x3fff_0000u128 << 96, 20, "3fff::/20 documentation"),
];

fn v4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn v6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    }
}

/// Name of the non-global range `ip` falls into, or `None` if `ip` is
/// global unicast. IPv4-mapped (`::ffff:a.b.c.d`) and NAT64
/// (`64:ff9b::/96`) addresses are judged by the IPv4 address they embed;
/// IPv4-compatible (`::a.b.c.d`) and everything else outside 2000::/3 is
/// refused outright.
pub fn blocked_range(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => blocked_v6(v6),
    }
}

/// True if `ip` is global unicast.
pub fn is_global(ip: IpAddr) -> bool {
    blocked_range(ip).is_none()
}

fn blocked_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let bits = u32::from(ip);
    V4_BLOCKED
        .iter()
        .find(|(net, prefix, _)| bits & v4_mask(*prefix) == *net)
        .map(|(_, _, name)| *name)
}

fn blocked_v6(ip: Ipv6Addr) -> Option<&'static str> {
    let bits = u128::from(ip);
    let high96 = bits >> 32;
    let embedded = Ipv4Addr::from((bits & 0xFFFF_FFFF) as u32);
    // ::ffff:0:0/96 IPv4-mapped.
    if high96 == 0x0000_0000_0000_0000_0000_ffff {
        return blocked_v4(embedded);
    }
    // ::/96 (unspecified, loopback, IPv4-compatible): never global.
    if high96 == 0 {
        return Some("::/96 unspecified, loopback or IPv4-compatible");
    }
    // 64:ff9b::/96 NAT64 well-known prefix: the embedded IPv4 decides.
    if high96 == 0x0064_ff9b_0000_0000_0000_0000 {
        return blocked_v4(embedded);
    }
    // Everything except 2000::/3 is non-global (ULA, link-local, site-local,
    // multicast, discard 100::/64, SRv6 5f00::/16, ...).
    if bits >> 125 != 0b001 {
        return Some("outside 2000::/3 (not global unicast)");
    }
    V6_BLOCKED
        .iter()
        .find(|(net, prefix, _)| bits & v6_mask(*prefix) == *net)
        .map(|(_, _, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test address")
    }

    #[test]
    fn every_blocked_v4_range_is_blocked_at_both_ends_and_inside() {
        for (net, prefix, name) in V4_BLOCKED {
            let first = Ipv4Addr::from(*net);
            let last = Ipv4Addr::from(*net | !v4_mask(*prefix));
            assert_eq!(blocked_v4(first), Some(*name), "{first}");
            assert_eq!(blocked_v4(last), Some(*name), "{last}");
        }
    }

    #[test]
    fn named_v4_cases() {
        for s in [
            "0.0.0.0",
            "0.255.255.255",
            "10.1.2.3",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "127.255.255.255",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_global(ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn v4_neighbours_of_blocked_ranges_are_global() {
        for s in [
            "1.1.1.1",
            "8.8.8.8",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.1",
            "192.0.3.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "223.255.255.255",
            "93.184.216.34",
        ] {
            assert!(is_global(ip(s)), "{s} must be global");
        }
    }

    #[test]
    fn every_blocked_v6_range_is_blocked_at_both_ends() {
        for (net, prefix, name) in V6_BLOCKED {
            let first = Ipv6Addr::from(*net);
            let last = Ipv6Addr::from(*net | !v6_mask(*prefix));
            assert_eq!(blocked_v6(first), Some(*name), "{first}");
            assert_eq!(blocked_v6(last), Some(*name), "{last}");
        }
    }

    #[test]
    fn named_v6_cases() {
        for s in [
            "::",
            "::1",
            "::2",
            "::8.8.8.8",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:192.168.1.1",
            "::ffff:0.0.0.0",
            "::ffff:255.255.255.255",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b::c0a8:1",
            "64:ff9b:1::1",
            "100::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "5f00::1",
            "2001::1",
            "2001:1::1",
            "2001:db8::1",
            "2002:7f00:1::1",
            "2002::",
            "3fff::1",
            "3fff:fff:ffff::1",
            "0100::",
        ] {
            assert!(!is_global(ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn global_v6_and_mapped_public() {
        for s in [
            "2606:4700:4700::1111",
            "2a00:1450:4001:81b::200e",
            "2001:4860:4860::8888",
            "2620:fe::fe",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2001:200::1",
        ] {
            assert!(is_global(ip(s)), "{s} must be global");
        }
    }
}

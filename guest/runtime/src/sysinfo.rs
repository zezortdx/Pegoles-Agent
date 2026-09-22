//! Minimal host facts for `GetSystemInfo`. Reads only:
//! `/etc/os-release`, `uname(2)`, `/proc/uptime`, `/proc/meminfo`,
//! hostname. Never environment, tokens, command lines, or home contents.

use pegoles_guest_proto::SystemInfo;

fn read_first_match(path: &str, prefix: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix(prefix)
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

fn field(path: &str, key: &str) -> Option<String> {
    read_first_match(path, &format!("{key}="))
}

pub fn collect() -> SystemInfo {
    let os = field("/etc/os-release", "ID").unwrap_or_else(|| "linux".into());
    let os_version = field("/etc/os-release", "VERSION_ID").unwrap_or_default();
    let kernel = uname_release();
    let arch = std::env::consts::ARCH.to_string();
    let hostname = hostname();
    SystemInfo {
        os,
        os_version,
        kernel,
        arch,
        hostname,
        runtime_version: pegoles_guest_proto::RUNTIME_VERSION.to_string(),
        protocol_version: pegoles_guest_proto::GUEST_PROTOCOL_VERSION,
        uptime_s: uptime_s(),
        cpu_count: cpu_count(),
        mem_total_mb: mem_total_mb(),
        // Phase 4 measurement facts: filled by the guest runtime stream.
        mem_available_mb: None,
        cpu_jiffies: None,
        process_rss: Vec::new(),
    }
}

fn uname_release() -> String {
    #[cfg(target_os = "linux")]
    {
        unsafe {
            let mut uts: libc::utsname = std::mem::zeroed();
            if libc::uname(&mut uts) != 0 {
                return String::new();
            }
            let bytes: Vec<u8> = uts.release.iter().map(|&c| c as u8).collect();
            let len = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..len]).into_owned()
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        "test-kernel".to_string()
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn uptime_s() -> Option<u64> {
    std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn cpu_count() -> Option<u32> {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .ok()
}

fn mem_total_mb() -> Option<u64> {
    let line = read_first_match("/proc/meminfo", "MemTotal:")?;
    line.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kb| kb / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysinfo_has_no_secrets() {
        let info = collect();
        let line = serde_json::to_string(&info).unwrap().to_lowercase();
        for forbidden in ["token", "secret", "key=", "password", "home/"] {
            assert!(!line.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(info.protocol_version, 1);
    }
}

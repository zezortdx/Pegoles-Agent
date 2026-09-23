//! Minimal host facts for `GetSystemInfo`. Reads only:
//! `/etc/os-release`, `uname(2)`, `/proc/uptime`, `/proc/meminfo`,
//! `/proc/stat`, hostname, and `comm` + `VmRSS` of ALLOWLISTED processes.
//! Never environment, tokens, command lines, or home contents.

use pegoles_guest_proto::{CpuJiffies, ProcessRss, SystemInfo, MAX_RSS_PROCESSES};

/// Processes whose resident memory is reported (kernel `comm`, which is
/// truncated to 15 bytes). Names only.
const RSS_ALLOWLIST: [&str; 4] = ["pegoles-guest-r", "weston", "foot", "pegoles-input-f"];

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
        mem_available_mb: meminfo_mb("MemAvailable:"),
        cpu_jiffies: std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| parse_cpu_jiffies(&s)),
        process_rss: allowlisted_rss(),
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
    meminfo_mb("MemTotal:")
}

fn meminfo_mb(key: &str) -> Option<u64> {
    let line = read_first_match("/proc/meminfo", key)?;
    line.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kb| kb / 1024)
}

/// Aggregate `cpu` line of /proc/stat: busy excludes idle + iowait.
pub fn parse_cpu_jiffies(stat: &str) -> Option<CpuJiffies> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    if fields.len() < 5 {
        return None;
    }
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields[3] + fields[4];
    Some(CpuJiffies {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// `VmRSS` in kB from a /proc/<pid>/status body.
pub fn parse_vm_rss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn allowlisted_rss() -> Vec<ProcessRss> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if out.len() >= MAX_RSS_PROCESSES {
            break;
        }
        let path = entry.path();
        let Ok(comm) = std::fs::read_to_string(path.join("comm")) else {
            continue;
        };
        let comm = comm.trim();
        if !RSS_ALLOWLIST.contains(&comm) {
            continue;
        }
        if let Some(rss_kb) = std::fs::read_to_string(path.join("status"))
            .ok()
            .and_then(|s| parse_vm_rss_kb(&s))
        {
            out.push(ProcessRss {
                name: comm.to_string(),
                rss_kb,
            });
        }
    }
    out
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

    #[test]
    fn parses_proc_stat_and_status() {
        let stat = "cpu  100 5 50 800 20 1 2 3 0 0\ncpu0 1 2 3 4 5 6 7 8\n";
        let j = parse_cpu_jiffies(stat).expect("cpu line");
        assert_eq!(j.total, 100 + 5 + 50 + 800 + 20 + 1 + 2 + 3);
        assert_eq!(j.busy, j.total - 800 - 20);
        assert!(parse_cpu_jiffies("intr 1 2").is_none());
        let status = "Name:\tweston\nVmRSS:\t   41234 kB\nThreads: 3\n";
        assert_eq!(parse_vm_rss_kb(status), Some(41234));
        assert_eq!(parse_vm_rss_kb("Name: x\n"), None);
    }
}

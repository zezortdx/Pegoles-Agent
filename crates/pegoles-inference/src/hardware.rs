//! Host hardware profile and memory measurement.
//!
//! Data first: this reports what the machine is and how much memory is
//! actually in use (system-wide and per process). Model choice policies
//! read these numbers; they do not live here.
//!
//! macOS uses `sysctl`, `host_statistics64` and `proc_pid_rusage`
//! (`phys_footprint` is the number Activity Monitor calls "Memory" and
//! includes Metal / unified-memory allocations attributed to a process).

use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HardwareProfile {
    pub os: String,
    pub arch: String,
    /// e.g. "Apple M4 Pro".
    pub chip: Option<String>,
    /// e.g. "Mac16,8".
    pub model_id: Option<String>,
    pub total_memory_bytes: u64,
    pub performance_cores: Option<u32>,
    pub efficiency_cores: Option<u32>,
    pub apple_silicon: bool,
    /// Metal is present on every Apple Silicon Mac; the worker confirms
    /// it at runtime (`hello.metal`).
    pub metal: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct SystemMemory {
    pub total_bytes: u64,
    /// App memory + wired + compressed (Activity Monitor "Memory Used").
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub wired_bytes: u64,
    pub compressed_bytes: u64,
    /// Kernel pressure level: 1 normal, 2 warning, 4 critical.
    pub pressure_level: Option<u32>,
    pub swap_used_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ProcessMemory {
    pub pid: i32,
    pub phys_footprint_bytes: u64,
    pub resident_bytes: u64,
    pub lifetime_max_phys_footprint_bytes: u64,
}

impl HardwareProfile {
    pub fn total_memory_gib(&self) -> f64 {
        self.total_memory_bytes as f64 / (1u64 << 30) as f64
    }
}

pub fn detect() -> HardwareProfile {
    let arch = std::env::consts::ARCH.to_string();
    let os = std::env::consts::OS.to_string();
    let apple_silicon = os == "macos" && arch == "aarch64";
    HardwareProfile {
        chip: sys::sysctl_string("machdep.cpu.brand_string"),
        model_id: sys::sysctl_string("hw.model"),
        total_memory_bytes: sys::sysctl_u64("hw.memsize").unwrap_or(0),
        performance_cores: sys::sysctl_u64("hw.perflevel0.physicalcpu").map(|v| v as u32),
        efficiency_cores: sys::sysctl_u64("hw.perflevel1.physicalcpu").map(|v| v as u32),
        metal: apple_silicon,
        apple_silicon,
        os,
        arch,
    }
}

pub fn system_memory() -> Option<SystemMemory> {
    sys::system_memory()
}

pub fn process_memory(pid: i32) -> Option<ProcessMemory> {
    sys::process_memory(pid)
}

#[cfg(target_os = "macos")]
mod sys {
    use super::{ProcessMemory, SystemMemory};
    use std::ffi::CString;

    pub fn sysctl_string(name: &str) -> Option<String> {
        let cname = CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        // SAFETY: size query with a null buffer, as documented.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || len == 0 || len > 4096 {
            return None;
        }
        let mut buf = vec![0u8; len];
        // SAFETY: buffer of `len` bytes, length passed in/out.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return None;
        }
        buf.truncate(len);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok().map(|s| s.trim().to_string())
    }

    pub fn sysctl_u64(name: &str) -> Option<u64> {
        let cname = CString::new(name).ok()?;
        let mut buf = [0u8; 8];
        let mut len: libc::size_t = buf.len();
        // SAFETY: 8-byte buffer; the kernel writes 4 or 8 bytes.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        match (rc, len) {
            (0, 8) => Some(u64::from_ne_bytes(buf)),
            (0, 4) => Some(u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64),
            _ => None,
        }
    }

    /// `struct vm_statistics64` (mach/vm_statistics.h).
    #[repr(C, align(8))]
    #[derive(Default)]
    struct VmStatistics64 {
        free_count: u32,
        active_count: u32,
        inactive_count: u32,
        wire_count: u32,
        zero_fill_count: u64,
        reactivations: u64,
        pageins: u64,
        pageouts: u64,
        faults: u64,
        cow_faults: u64,
        lookups: u64,
        hits: u64,
        purges: u64,
        purgeable_count: u32,
        speculative_count: u32,
        decompressions: u64,
        compressions: u64,
        swapins: u64,
        swapouts: u64,
        compressor_page_count: u32,
        throttled_count: u32,
        external_page_count: u32,
        internal_page_count: u32,
        total_uncompressed_pages_in_compressor: u64,
    }

    const HOST_VM_INFO64: i32 = 4;

    /// `struct xsw_usage` (sys/sysctl.h).
    #[repr(C)]
    #[derive(Default)]
    struct XswUsage {
        total: u64,
        avail: u64,
        used: u64,
        pagesize: u32,
        encrypted: u32,
    }

    extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics64(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV4) -> i32;
    }

    pub fn system_memory() -> Option<SystemMemory> {
        let total = sysctl_u64("hw.memsize")?;
        let page = sysctl_u64("hw.pagesize").unwrap_or(16384);
        let mut stats = VmStatistics64::default();
        let mut count = (std::mem::size_of::<VmStatistics64>() / 4) as u32;
        // SAFETY: struct matches vm_statistics64; count is its size in
        // integer_t units, as HOST_VM_INFO64_COUNT.
        let rc = unsafe {
            host_statistics64(
                mach_host_self(),
                HOST_VM_INFO64,
                (&mut stats as *mut VmStatistics64).cast(),
                &mut count,
            )
        };
        if rc != 0 {
            return None;
        }
        let app = (stats.internal_page_count as u64).saturating_sub(stats.purgeable_count as u64);
        let wired = stats.wire_count as u64 * page;
        let compressed = stats.compressor_page_count as u64 * page;
        let used = (app * page + wired + compressed).min(total);
        Some(SystemMemory {
            total_bytes: total,
            used_bytes: used,
            free_bytes: stats.free_count as u64 * page,
            wired_bytes: wired,
            compressed_bytes: compressed,
            pressure_level: sysctl_u64("kern.memorystatus_vm_pressure_level").map(|v| v as u32),
            swap_used_bytes: swap_used(),
        })
    }

    fn swap_used() -> Option<u64> {
        let cname = CString::new("vm.swapusage").ok()?;
        let mut xsw = XswUsage::default();
        let mut len: libc::size_t = std::mem::size_of::<XswUsage>();
        // SAFETY: buffer is a correctly sized xsw_usage.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                (&mut xsw as *mut XswUsage).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0).then_some(xsw.used)
    }

    /// `struct rusage_info_v4` (sys/resource.h): 16-byte uuid then 35 u64.
    #[repr(C)]
    struct RusageInfoV4 {
        uuid: [u8; 16],
        fields: [u64; 35],
    }
    const RUSAGE_INFO_V4: i32 = 4;
    const RI_RESIDENT_SIZE: usize = 6;
    const RI_PHYS_FOOTPRINT: usize = 7;
    const RI_LIFETIME_MAX_PHYS_FOOTPRINT: usize = 28;

    pub fn process_memory(pid: i32) -> Option<ProcessMemory> {
        let mut info = RusageInfoV4 {
            uuid: [0; 16],
            fields: [0; 35],
        };
        // SAFETY: buffer is a rusage_info_v4 for flavor RUSAGE_INFO_V4.
        let rc = unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V4, &mut info) };
        if rc != 0 {
            return None;
        }
        Some(ProcessMemory {
            pid,
            phys_footprint_bytes: info.fields[RI_PHYS_FOOTPRINT],
            resident_bytes: info.fields[RI_RESIDENT_SIZE],
            lifetime_max_phys_footprint_bytes: info.fields[RI_LIFETIME_MAX_PHYS_FOOTPRINT],
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod sys {
    use super::{ProcessMemory, SystemMemory};

    pub fn sysctl_string(_name: &str) -> Option<String> {
        None
    }
    pub fn sysctl_u64(_name: &str) -> Option<u64> {
        None
    }
    pub fn system_memory() -> Option<SystemMemory> {
        None
    }
    pub fn process_memory(_pid: i32) -> Option<ProcessMemory> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn detects_this_mac_and_measures_memory() {
        let hw = detect();
        assert!(hw.total_memory_bytes > 1 << 30);
        let mem = system_memory().expect("host_statistics64");
        assert_eq!(mem.total_bytes, hw.total_memory_bytes);
        assert!(mem.used_bytes > 0 && mem.used_bytes <= mem.total_bytes);
        let me = process_memory(std::process::id() as i32).expect("proc_pid_rusage");
        assert!(me.phys_footprint_bytes > 0);
        assert!(me.lifetime_max_phys_footprint_bytes >= me.phys_footprint_bytes / 2);
    }

    #[test]
    fn missing_process_is_none() {
        assert!(process_memory(-7).is_none());
    }
}

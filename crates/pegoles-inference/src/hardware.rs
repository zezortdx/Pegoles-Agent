//! Host hardware profile and memory measurement.
//!
//! Data first: this reports what the machine is and how much memory is
//! actually in use (system-wide and per process). Model choice policies
//! read these numbers; they do not live here.
//!
//! macOS uses `sysctl`, `host_statistics64` and `proc_pid_rusage`
//! (`phys_footprint` is the number Activity Monitor calls "Memory" and
//! includes Metal / unified-memory allocations attributed to a process).
//! Windows uses `GlobalMemoryStatusEx`, the process's private commit
//! (Task Manager's "Commit size") and DXGI for the GPU list.

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

#[cfg(not(windows))]
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

#[cfg(windows)]
pub fn detect() -> HardwareProfile {
    HardwareProfile {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        chip: sys::cpu_name(),
        model_id: None,
        total_memory_bytes: sys::system_memory().map_or(0, |m| m.total_bytes),
        performance_cores: None,
        efficiency_cores: None,
        apple_silicon: false,
        metal: false,
    }
}

pub fn system_memory() -> Option<SystemMemory> {
    sys::system_memory()
}

/// Which llama.cpp backend a Windows PC can use.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcceleratorKind {
    Cuda,
    Vulkan,
    Metal,
    Cpu,
}

/// The expected accelerator, before the worker confirms it at load.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Acceleration {
    pub kind: AcceleratorKind,
    /// The GPU's name as Windows reports it, when there is one.
    pub device: Option<String>,
    pub technical: String,
}

/// One display adapter (DXGI).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuAdapter {
    pub name: String,
    pub vendor_id: u32,
    pub dedicated_bytes: u64,
    pub shared_bytes: u64,
}

/// Pure choice: Vulkan on the best hardware adapter when the Vulkan
/// loader is installed (every current NVIDIA, AMD and Intel driver ships
/// it), otherwise the CPU. Pegoles ships llama.cpp's Vulkan and CPU
/// backends only (no CUDA runtime: it would add gigabytes), so a GPU
/// without a Vulkan driver runs on the CPU.
pub fn choose_acceleration(adapters: &[GpuAdapter], vulkan_loader: bool) -> Acceleration {
    let best = adapters
        .iter()
        .filter(|a| a.vendor_id != MICROSOFT_VENDOR)
        .max_by_key(|a| (a.dedicated_bytes, a.shared_bytes));
    let listed = if adapters.is_empty() {
        "no display adapters reported".to_string()
    } else {
        adapters
            .iter()
            .map(|a| {
                format!(
                    "{} (vendor {:04x}, {} MB dedicated)",
                    a.name,
                    a.vendor_id,
                    a.dedicated_bytes >> 20
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    match best {
        Some(gpu) if vulkan_loader => Acceleration {
            kind: AcceleratorKind::Vulkan,
            device: Some(gpu.name.clone()),
            technical: format!("Vulkan on {}; adapters: {listed}", gpu.name),
        },
        Some(gpu) => Acceleration {
            kind: AcceleratorKind::Cpu,
            device: Some(gpu.name.clone()),
            technical: format!(
                "no Vulkan loader (vulkan-1.dll) installed; CPU. Adapters: {listed}"
            ),
        },
        None => Acceleration {
            kind: AcceleratorKind::Cpu,
            device: None,
            technical: format!("no hardware GPU; CPU. Adapters: {listed}"),
        },
    }
}

/// PCI vendor id of Microsoft's software adapters (Basic Render Driver).
const MICROSOFT_VENDOR: u32 = 0x1414;

#[cfg(windows)]
pub fn windows_acceleration() -> Acceleration {
    choose_acceleration(&sys::gpu_adapters(), sys::vulkan_loader_present())
}

/// The OS release as people know it ("15.5" on macOS). None when the OS
/// does not say (other hosts report their version elsewhere).
pub fn os_product_version() -> Option<String> {
    sys::sysctl_string("kern.osproductversion")
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

#[cfg(windows)]
mod sys {
    use super::{GpuAdapter, ProcessMemory, SystemMemory};
    use windows::core::w;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    use windows::Win32::System::SystemInformation::{
        GetSystemDirectoryW, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// Longest GPU / CPU name kept.
    const MAX_NAME: usize = 120;
    /// Adapters looked at (DXGI lists hardware first).
    const MAX_ADAPTERS: u32 = 8;

    pub fn sysctl_string(_name: &str) -> Option<String> {
        None
    }

    pub fn cpu_name() -> Option<String> {
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffer and its byte length describe the same array.
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"),
                w!("ProcessorNameString"),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        }
        .ok()
        .ok()?;
        let name = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
        let name = name.trim_matches(char::from(0)).trim();
        (!name.is_empty()).then(|| name.chars().take(MAX_NAME).collect())
    }

    pub fn system_memory() -> Option<SystemMemory> {
        let mut st = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: dwLength set as documented.
        unsafe { GlobalMemoryStatusEx(&mut st) }.ok()?;
        Some(SystemMemory {
            total_bytes: st.ullTotalPhys,
            used_bytes: st.ullTotalPhys.saturating_sub(st.ullAvailPhys),
            free_bytes: st.ullAvailPhys,
            wired_bytes: 0,
            compressed_bytes: 0,
            pressure_level: None,
            swap_used_bytes: None,
        })
    }

    pub fn process_memory(pid: i32) -> Option<ProcessMemory> {
        let id = u32::try_from(pid).ok()?;
        // SAFETY: query-only handle, closed below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id) }.ok()?;
        let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
        // SAFETY: the EX struct starts with the base struct; cb is its size.
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                process,
                (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )
        }
        .as_bool();
        // SAFETY: the handle came from OpenProcess.
        let _ = unsafe { CloseHandle(process) };
        ok.then_some(ProcessMemory {
            pid,
            phys_footprint_bytes: c.PrivateUsage as u64,
            resident_bytes: c.WorkingSetSize as u64,
            lifetime_max_phys_footprint_bytes: c.PeakPagefileUsage as u64,
        })
    }

    pub fn gpu_adapters() -> Vec<GpuAdapter> {
        // SAFETY: plain COM factory creation.
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..MAX_ADAPTERS {
            // SAFETY: index enumeration; stops at DXGI_ERROR_NOT_FOUND.
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
                break;
            };
            // SAFETY: valid adapter.
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            out.push(GpuAdapter {
                name: String::from_utf16_lossy(&desc.Description[..end])
                    .trim()
                    .chars()
                    .take(MAX_NAME)
                    .collect(),
                vendor_id: desc.VendorId,
                dedicated_bytes: desc.DedicatedVideoMemory as u64,
                shared_bytes: desc.SharedSystemMemory as u64,
            });
        }
        out
    }

    /// `%SystemRoot%\System32\vulkan-1.dll`: the Khronos loader GPU drivers
    /// install. Only its presence is checked; nothing is loaded here.
    pub fn vulkan_loader_present() -> bool {
        let mut buf = [0u16; 260];
        // SAFETY: buffer sized as passed.
        let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 || n >= buf.len() {
            return false;
        }
        let dir = String::from_utf16_lossy(&buf[..n]);
        std::path::Path::new(&dir).join("vulkan-1.dll").is_file()
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
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
    fn acceleration_prefers_the_biggest_real_gpu_with_vulkan() {
        let gpu = |name: &str, vendor, dedicated| GpuAdapter {
            name: name.into(),
            vendor_id: vendor,
            dedicated_bytes: dedicated,
            shared_bytes: 8 << 30,
        };
        let adapters = [
            gpu("Intel(R) UHD Graphics", 0x8086, 128 << 20),
            gpu("NVIDIA GeForce RTX 3060", 0x10de, 12 << 30),
            gpu("Microsoft Basic Render Driver", MICROSOFT_VENDOR, 0),
        ];
        let a = choose_acceleration(&adapters, true);
        assert_eq!(a.kind, AcceleratorKind::Vulkan);
        assert_eq!(a.device.as_deref(), Some("NVIDIA GeForce RTX 3060"));
        let no_loader = choose_acceleration(&adapters, false);
        assert_eq!(no_loader.kind, AcceleratorKind::Cpu);
        assert!(no_loader.technical.contains("vulkan-1.dll"));
        let basic_only = choose_acceleration(&adapters[2..], true);
        assert_eq!(
            (basic_only.kind, basic_only.device),
            (AcceleratorKind::Cpu, None)
        );
        assert_eq!(choose_acceleration(&[], true).kind, AcceleratorKind::Cpu);
    }

    #[test]
    fn missing_process_is_none() {
        assert!(process_memory(-7).is_none());
    }
}

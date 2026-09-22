//! ResourceGovernor: centralized host resource policy (Phase 3.6 §40).
//!
//! Every resource heuristic lives HERE — never scattered across backends,
//! UI, or agent code. The governor inspects the host (memory, CPUs,
//! architecture, pressure/load where safely obtainable) and recommends a
//! Pegoles Computer configuration plus a performance profile.
//!
//! Conservative by design: it must never recommend so much VM memory that
//! the host starts swapping aggressively, nor so many vCPUs that the host
//! starves. All bounds from `config.rs` still apply on top (validation).

use serde::{Deserialize, Serialize};

/// Performance profile: Eco (minimal footprint), Balanced (default),
/// Performance (stronger hosts), Custom (manual, still validated).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceProfile {
    Eco,
    #[default]
    Balanced,
    Performance,
    Custom,
}

/// Host power state, where reliably detectable. Unknown when it cannot
/// be (no fragile platform assumptions, no new daemons).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

/// Host memory pressure advisory. Detection points are platform-specific
/// and land incrementally; the default is Unknown (never assumed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPressure {
    Normal,
    Elevated,
    Critical,
    Unknown,
}

/// Snapshot of what the governor may consider. Constructed from live
/// detection (`HostResources::detect()`) or injected by tests/bench.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostResources {
    /// Physical RAM in MiB (0 = unknown).
    pub total_ram_mb: u64,
    /// Free+reclaimable RAM in MiB (0 = unknown).
    pub available_ram_mb: u64,
    /// Logical CPU cores (0 = unknown).
    pub cpu_cores: u32,
    pub power: PowerSource,
    pub pressure: MemoryPressure,
}

/// Governor output: concrete, validated-against-bounds numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerRecommendation {
    pub vcpus: u8,
    pub memory_mb: u32,
    pub profile: PerformanceProfile,
    /// Human-readable caveats, e.g. "host has 8 GB: staying near 1 GB".
    pub warnings: Vec<String>,
}

/// Idle lifecycle concepts (Phase 3.6 §46: model now, automate later).
/// No automatic destructive behavior ships: policies are data for a
/// future scheduler, always overridable per task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleState {
    Active,
    Idle,
    Paused,
    Stopped,
}

/// Thresholds for future idle policies. All disabled by default
/// (`None`); a scheduler may opt in explicitly per computer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdlePolicy {
    pub idle_after_secs: Option<u64>,
    pub pause_after_secs: Option<u64>,
    pub stop_after_secs: Option<u64>,
}

/// Recommend a computer configuration. Rules (conservative, documented):
/// - vCPUs: min(cores-1, 4), clamped to [1, 4]; Eco forces 1; Performance
///   allows up to min(cores-1, 8); Custom is validated, not invented.
/// - RAM tiers by total host RAM: <=8G -> ~1G, <=16G -> ~1.5G,
///   else ~2G; Eco halves toward the 512 MB floor; Performance adds 50%
///   capped so the guest never exceeds 1/4 of host RAM.
/// - Battery + Eco/Balanced: prefer the lower tier (documented nudge).
/// - Unknown host values degrade to the safe static default (2/1536).
pub fn recommend(
    host: &HostResources,
    profile: PerformanceProfile,
    custom: Option<(u8, u32)>,
) -> ComputerRecommendation {
    let mut warnings = Vec::new();
    if host.total_ram_mb == 0 || host.cpu_cores == 0 {
        warnings.push("host resources unknown: using safe static default".to_string());
        return ComputerRecommendation {
            vcpus: 2,
            memory_mb: 1536,
            profile,
            warnings,
        };
    }
    if let Some((want_vcpus, want_ram)) = custom {
        // Custom is validated against safe bounds, never blindly applied.
        // Like every profile, one core stays with the host.
        let vcpu_cap = host.cpu_cores.saturating_sub(1).clamp(1, 8) as u8;
        let vcpus = want_vcpus.clamp(1, vcpu_cap);
        let ram_cap = (host.total_ram_mb / 4).max(512) as u32;
        let memory_mb = want_ram.clamp(512, ram_cap);
        if vcpus != want_vcpus || memory_mb != want_ram {
            warnings.push("custom values clamped to safe host bounds".to_string());
        }
        return ComputerRecommendation {
            vcpus,
            memory_mb,
            profile: PerformanceProfile::Custom,
            warnings,
        };
    }

    let cores_budget = host.cpu_cores.saturating_sub(1).max(1);
    let mut vcpus = cores_budget.clamp(1, 4) as u8;
    let mut ram_mb: u32 = if host.total_ram_mb <= 8 * 1024 {
        1024
    } else if host.total_ram_mb <= 16 * 1024 {
        1536
    } else {
        2048
    };
    if host.total_ram_mb <= 8 * 1024 {
        warnings.push("host has <= 8 GB: staying near 1 GB for the guest".to_string());
    }

    match profile {
        PerformanceProfile::Eco => {
            vcpus = 1;
            ram_mb = (ram_mb / 2).max(512);
            if matches!(host.power, PowerSource::Battery) {
                warnings.push("battery + eco: minimum footprint".to_string());
            }
        }
        PerformanceProfile::Balanced => {}
        PerformanceProfile::Performance => {
            vcpus = cores_budget.clamp(1, 8) as u8;
            ram_mb = ((ram_mb as u64 * 3 / 2).min(host.total_ram_mb / 4).max(512)) as u32;
        }
        PerformanceProfile::Custom => {}
    }
    // Hard ceiling: never more than a quarter of host RAM.
    let cap = (host.total_ram_mb / 4).max(512) as u32;
    if ram_mb > cap {
        ram_mb = cap;
        warnings.push("guest RAM capped at 1/4 of host RAM".to_string());
    }
    ComputerRecommendation {
        vcpus: vcpus.max(1),
        memory_mb: ram_mb.max(512),
        profile,
        warnings,
    }
}

/// UI effects tier (Phase 4). Functionality never depends on the tier:
/// Minimal is still premium (materials become opaque surfaces, ambient
/// motion sleeps), never "broken". The UI may override it in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectsTier {
    /// Lowest cost: no backdrop blur, no ambient animation.
    Minimal,
    /// Less blur/glow, fewer ambient animations, shorter trails.
    Reduced,
    /// Everything on (still no idle full-screen loops, ever).
    Full,
}

/// Why a tier was recommended (machine-readable, snake_case on the wire).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectsReason {
    /// Host RAM/cores unknown: the safe middle tier.
    HostUnknown,
    /// <= 4 GB RAM.
    VeryLowMemory,
    /// <= 2 logical cores.
    VeryFewCores,
    /// Host reports critical memory pressure.
    MemoryPressureCritical,
    /// <= 8 GB RAM (the low-end fixture).
    LowMemory,
    /// <= 4 logical cores.
    FewCores,
    /// Eco profile caps effects at Reduced.
    EcoProfile,
    /// Running on battery caps effects at Reduced.
    OnBattery,
    /// The OS asks for reduced motion. INFORMATIONAL: motion is handled
    /// by the UI independently of the tier and accessibility settings are
    /// never changed by Pegoles.
    PrefersReducedMotion,
    /// Nothing limits effects on this host.
    CapableHost,
}

/// Governor output for the UI effects tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectsRecommendation {
    pub tier: EffectsTier,
    pub reasons: Vec<EffectsReason>,
}

/// Recommend a UI effects tier. Rules (conservative, documented):
/// - Unknown host (RAM or cores = 0) -> Reduced (safe middle).
/// - <= 4 GB RAM, or <= 2 cores, or MemoryPressure::Critical -> Minimal.
/// - <= 8 GB RAM, or <= 4 cores (low-end fixture 8 GB / 4 cores) -> Reduced.
/// - Eco profile -> at most Reduced.
/// - Battery (only when actually detected) -> at most Reduced.
/// - Otherwise Full.
///
/// `prefers_reduced_motion` is reported as a reason but NEVER changes the
/// tier: reduced motion is an accessibility preference the UI honors on
/// its own (brightness/opacity instead of movement), orthogonal to cost.
pub fn recommend_effects(
    host: &HostResources,
    profile: PerformanceProfile,
    prefers_reduced_motion: Option<bool>,
) -> EffectsRecommendation {
    let mut reasons = Vec::new();
    let mut tier = EffectsTier::Full;
    let mut cap = |limit: EffectsTier, reason: EffectsReason, reasons: &mut Vec<EffectsReason>| {
        tier = tier.min(limit);
        reasons.push(reason);
    };
    if host.total_ram_mb == 0 || host.cpu_cores == 0 {
        cap(
            EffectsTier::Reduced,
            EffectsReason::HostUnknown,
            &mut reasons,
        );
    } else {
        if host.total_ram_mb <= 4 * 1024 {
            cap(
                EffectsTier::Minimal,
                EffectsReason::VeryLowMemory,
                &mut reasons,
            );
        } else if host.total_ram_mb <= 8 * 1024 {
            cap(EffectsTier::Reduced, EffectsReason::LowMemory, &mut reasons);
        }
        if host.cpu_cores <= 2 {
            cap(
                EffectsTier::Minimal,
                EffectsReason::VeryFewCores,
                &mut reasons,
            );
        } else if host.cpu_cores <= 4 {
            cap(EffectsTier::Reduced, EffectsReason::FewCores, &mut reasons);
        }
    }
    if host.pressure == MemoryPressure::Critical {
        cap(
            EffectsTier::Minimal,
            EffectsReason::MemoryPressureCritical,
            &mut reasons,
        );
    }
    if profile == PerformanceProfile::Eco {
        cap(
            EffectsTier::Reduced,
            EffectsReason::EcoProfile,
            &mut reasons,
        );
    }
    if host.power == PowerSource::Battery {
        cap(EffectsTier::Reduced, EffectsReason::OnBattery, &mut reasons);
    }
    if reasons.is_empty() {
        reasons.push(EffectsReason::CapableHost);
    }
    if prefers_reduced_motion == Some(true) {
        reasons.push(EffectsReason::PrefersReducedMotion);
    }
    EffectsRecommendation { tier, reasons }
}

/// Balloon reclaim policy (pure computation). Enforcement points differ
/// per hypervisor and some don't exist yet:
/// - macOS: the virtio balloon device is attached (hypervisor-managed);
///   Vz exposes no public host-driven target API, so there is nothing to
///   call — the policy output is recorded for future use.
/// - Windows (designed): HCS modify-memory converges toward the target.
///
/// Given an optional observed guest usage, compute a safe target.
///
/// Never below the 512 MB floor, never above assigned. Hysteresis via
/// callers (call at most on lifecycle transitions, not in a loop).
pub struct BalloonPolicy {
    /// Never suggest below this, even for idle guests.
    pub floor_mb: u64,
}

impl Default for BalloonPolicy {
    fn default() -> Self {
        Self { floor_mb: 512 }
    }
}

impl BalloonPolicy {
    /// Target guest size in MB. `None` usage (unobserved) means "no
    /// opinion" → returns assigned (no shrink without evidence).
    pub fn reclaim_target_mb(&self, guest_used_mb: Option<u64>, assigned_mb: u32) -> u64 {
        match guest_used_mb {
            None => assigned_mb as u64,
            Some(used) => {
                // Keep 25% headroom over observed use, floored and capped.
                let target = used + used / 4;
                target.clamp(self.floor_mb, assigned_mb as u64)
            }
        }
    }
}
/// Centralized host detection. macOS: sysctl (hw.memsize, hw.ncpu) plus
/// the public IOKit power-sources estimate for AC vs battery — stable,
/// no daemons, no spawns. Other platforms: conservative Unknowns
/// (detection points documented in PERFORMANCE.md). Memory pressure stays
/// Unknown everywhere until a reliable source lands.
pub fn detect_host_resources() -> HostResources {
    #[cfg(target_os = "macos")]
    {
        detect_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        HostResources {
            total_ram_mb: 0,
            available_ram_mb: 0,
            cpu_cores: 0,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_macos() -> HostResources {
    let total_ram_mb = sysctl_u64("hw.memsize")
        .map(|b| b / (1024 * 1024))
        .unwrap_or(0);
    let cpu_cores = sysctl_u32("hw.ncpu").unwrap_or(0);
    HostResources {
        total_ram_mb,
        // Free-memory sysctls (vm.loadavg/vm_stat) are snapshots, not
        // budgets; the governor sizes from TOTAL ram and caps at 1/4, so
        // available is intentionally left Unknown rather than sampled.
        available_ram_mb: 0,
        cpu_cores,
        power: detect_power_source_macos(),
        pressure: MemoryPressure::Unknown,
    }
}

/// `kIOPSTimeRemainingUnlimited` (IOKit/ps/IOPowerSources.h).
#[cfg(any(target_os = "macos", test))]
const IOPS_TIME_REMAINING_UNLIMITED: f64 = -2.0;

/// Map the public IOKit estimate to a power source: `-2.0`
/// (kIOPSTimeRemainingUnlimited) = attached to an unlimited source (AC,
/// also desktops without a battery); a positive estimate = on a limited
/// source (battery/UPS); `-1.0` (still calculating / cannot determine)
/// and anything else = Unknown — never guessed.
#[cfg(any(target_os = "macos", test))]
fn power_source_from_estimate(estimate: f64) -> PowerSource {
    if estimate == IOPS_TIME_REMAINING_UNLIMITED {
        PowerSource::Ac
    } else if estimate.is_finite() && estimate > 0.0 {
        PowerSource::Battery
    } else {
        PowerSource::Unknown
    }
}

#[cfg(target_os = "macos")]
fn detect_power_source_macos() -> PowerSource {
    // SAFETY: documented public IOKit function; takes no arguments,
    // returns a plain double, allocates nothing the caller must free.
    let estimate = unsafe { IOPSGetTimeRemainingEstimate() };
    power_source_from_estimate(estimate)
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// CFTimeInterval (double) IOPSGetTimeRemainingEstimate(void).
    fn IOPSGetTimeRemainingEstimate() -> f64;
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let cname = std::ffi::CString::new(name).ok()?;
    // SAFETY: sysctlbyname with a u64 buffer is the documented pattern.
    let ret = unsafe {
        libc_sysctlbyname(
            cname.as_ptr(),
            &mut value as *mut _ as *mut std::ffi::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret == 0 {
        Some(value)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &str) -> Option<u32> {
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let cname = std::ffi::CString::new(name).ok()?;
    let ret = unsafe {
        libc_sysctlbyname(
            cname.as_ptr(),
            &mut value as *mut _ as *mut std::ffi::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret == 0 {
        Some(value)
    } else {
        None
    }
}

// Minimal sysctlbyname binding (avoids a libc dependency for two calls;
// links against the default system libraries).
#[cfg(target_os = "macos")]
extern "C" {
    #[link_name = "sysctlbyname"]
    fn libc_sysctlbyname(
        name: *const std::os::raw::c_char,
        oldp: *mut std::ffi::c_void,
        oldlenp: *mut usize,
        newp: *mut std::ffi::c_void,
        newlen: usize,
    ) -> std::os::raw::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_8gb() -> HostResources {
        HostResources {
            total_ram_mb: 8192,
            available_ram_mb: 0,
            cpu_cores: 4,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        }
    }
    fn host_16gb() -> HostResources {
        HostResources {
            total_ram_mb: 16384,
            available_ram_mb: 0,
            cpu_cores: 8,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        }
    }
    fn host_32gb() -> HostResources {
        HostResources {
            total_ram_mb: 32768,
            available_ram_mb: 0,
            cpu_cores: 12,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        }
    }

    #[test]
    fn low_end_8gb_stays_near_1gb() {
        let r = recommend(&host_8gb(), PerformanceProfile::Balanced, None);
        assert_eq!((r.vcpus, r.memory_mb), (3, 1024));
        assert!(!r.warnings.is_empty());
    }

    #[test]
    fn mid_16gb_gets_default() {
        let r = recommend(&host_16gb(), PerformanceProfile::Balanced, None);
        assert_eq!((r.vcpus, r.memory_mb), (4, 1536));
    }

    #[test]
    fn big_32gb_gets_2gb_never_host_starving() {
        let r = recommend(&host_32gb(), PerformanceProfile::Balanced, None);
        assert_eq!((r.vcpus, r.memory_mb), (4, 2048));
        // Never more than a quarter of host RAM, never all cores.
        assert!(r.memory_mb as u64 <= 32768 / 4);
        assert!(r.vcpus < 12);
    }

    #[test]
    fn eco_is_minimal_and_battery_noted() {
        let mut host = host_16gb();
        host.power = PowerSource::Battery;
        let r = recommend(&host, PerformanceProfile::Eco, None);
        assert_eq!((r.vcpus, r.memory_mb), (1, 768));
        assert!(r.warnings.iter().any(|w| w.contains("battery")));
    }

    #[test]
    fn performance_stays_bounded() {
        let r = recommend(&host_32gb(), PerformanceProfile::Performance, None);
        assert!(r.memory_mb <= 8192);
        assert!(r.vcpus <= 8);
    }

    #[test]
    fn custom_clamped_to_safe_bounds() {
        let r = recommend(&host_8gb(), PerformanceProfile::Custom, Some((64, 1 << 20)));
        assert_eq!(r.vcpus, 3); // min(cores-1, 8)
        assert_eq!(r.memory_mb, 2048); // 1/4 of 8 GB
        assert!(!r.warnings.is_empty());
        assert_eq!(r.profile, PerformanceProfile::Custom);
    }

    #[test]
    fn unknown_host_gets_safe_default() {
        let unknown = HostResources {
            total_ram_mb: 0,
            available_ram_mb: 0,
            cpu_cores: 0,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        };
        let r = recommend(&unknown, PerformanceProfile::Balanced, None);
        assert_eq!((r.vcpus, r.memory_mb), (2, 1536));
    }

    #[test]
    fn balloon_never_shrinks_without_evidence() {
        let p = BalloonPolicy::default();
        assert_eq!(p.reclaim_target_mb(None, 1536), 1536);
        assert_eq!(p.reclaim_target_mb(Some(400), 1536), 512); // floor
        assert_eq!(p.reclaim_target_mb(Some(1000), 1536), 1250); // +25%
        assert_eq!(p.reclaim_target_mb(Some(2000), 1536), 1536); // capped
    }

    #[test]
    fn idle_policy_defaults_to_manual() {
        let p = IdlePolicy::default();
        assert_eq!(p.idle_after_secs, None);
        assert_eq!(p.pause_after_secs, None);
        assert_eq!(p.stop_after_secs, None);
    }

    #[test]
    fn detect_returns_sane_values() {
        let host = detect_host_resources();
        #[cfg(target_os = "macos")]
        {
            assert!(host.total_ram_mb >= 1024, "got {}", host.total_ram_mb);
            assert!(host.cpu_cores >= 1, "got {}", host.cpu_cores);
        }
        // Power comes from IOKit on macOS (any honest value); elsewhere it
        // stays Unknown. Pressure is Unknown until detection lands.
        #[cfg(not(target_os = "macos"))]
        assert_eq!(host.power, PowerSource::Unknown);
        assert_eq!(host.pressure, MemoryPressure::Unknown);
    }

    #[test]
    fn power_estimate_mapping_never_guesses() {
        assert_eq!(power_source_from_estimate(-2.0), PowerSource::Ac);
        assert_eq!(power_source_from_estimate(3600.0), PowerSource::Battery);
        assert_eq!(power_source_from_estimate(-1.0), PowerSource::Unknown);
        assert_eq!(power_source_from_estimate(0.0), PowerSource::Unknown);
        assert_eq!(power_source_from_estimate(f64::NAN), PowerSource::Unknown);
    }

    fn host(ram_gb: u64, cores: u32) -> HostResources {
        HostResources {
            total_ram_mb: ram_gb * 1024,
            available_ram_mb: 0,
            cpu_cores: cores,
            power: PowerSource::Unknown,
            pressure: MemoryPressure::Unknown,
        }
    }

    #[test]
    fn effects_low_end_fixture_is_reduced() {
        let r = recommend_effects(&host(8, 4), PerformanceProfile::Balanced, None);
        assert_eq!(r.tier, EffectsTier::Reduced);
        assert!(r.reasons.contains(&EffectsReason::LowMemory));
        assert!(r.reasons.contains(&EffectsReason::FewCores));
    }

    #[test]
    fn effects_capable_hosts_get_full() {
        for (ram, cores) in [(16, 8), (24, 12), (32, 12)] {
            let r = recommend_effects(&host(ram, cores), PerformanceProfile::Balanced, None);
            assert_eq!(r.tier, EffectsTier::Full, "{ram} GB / {cores} cores");
            assert_eq!(r.reasons, vec![EffectsReason::CapableHost]);
        }
    }

    #[test]
    fn effects_very_low_end_is_minimal() {
        let r = recommend_effects(&host(4, 8), PerformanceProfile::Balanced, None);
        assert_eq!(r.tier, EffectsTier::Minimal);
        let r = recommend_effects(&host(16, 2), PerformanceProfile::Balanced, None);
        assert_eq!(r.tier, EffectsTier::Minimal);
        let mut pressured = host(32, 12);
        pressured.pressure = MemoryPressure::Critical;
        let r = recommend_effects(&pressured, PerformanceProfile::Performance, None);
        assert_eq!(r.tier, EffectsTier::Minimal);
        assert!(r.reasons.contains(&EffectsReason::MemoryPressureCritical));
    }

    #[test]
    fn effects_unknown_host_is_reduced() {
        let r = recommend_effects(&host(0, 0), PerformanceProfile::Balanced, None);
        assert_eq!(r.tier, EffectsTier::Reduced);
        assert_eq!(r.reasons, vec![EffectsReason::HostUnknown]);
    }

    #[test]
    fn effects_battery_and_eco_cap_at_reduced() {
        let mut on_battery = host(24, 12);
        on_battery.power = PowerSource::Battery;
        let r = recommend_effects(&on_battery, PerformanceProfile::Balanced, None);
        assert_eq!(r.tier, EffectsTier::Reduced);
        assert!(r.reasons.contains(&EffectsReason::OnBattery));
        let r = recommend_effects(&host(24, 12), PerformanceProfile::Eco, None);
        assert_eq!(r.tier, EffectsTier::Reduced);
        assert!(r.reasons.contains(&EffectsReason::EcoProfile));
        // AC power is not a cap.
        let mut on_ac = host(24, 12);
        on_ac.power = PowerSource::Ac;
        assert_eq!(
            recommend_effects(&on_ac, PerformanceProfile::Balanced, None).tier,
            EffectsTier::Full
        );
        // Caps never raise a lower tier.
        let mut tiny = host(4, 2);
        tiny.power = PowerSource::Battery;
        assert_eq!(
            recommend_effects(&tiny, PerformanceProfile::Eco, None).tier,
            EffectsTier::Minimal
        );
    }

    #[test]
    fn reduced_motion_is_reported_never_applied() {
        let with = recommend_effects(&host(24, 12), PerformanceProfile::Balanced, Some(true));
        let without = recommend_effects(&host(24, 12), PerformanceProfile::Balanced, Some(false));
        assert_eq!(with.tier, EffectsTier::Full);
        assert_eq!(with.tier, without.tier);
        assert!(with.reasons.contains(&EffectsReason::PrefersReducedMotion));
        assert!(!without
            .reasons
            .contains(&EffectsReason::PrefersReducedMotion));
    }

    #[test]
    fn effects_wire_names_are_snake_case() {
        let r = recommend_effects(&host(8, 4), PerformanceProfile::Balanced, Some(true));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["tier"], "reduced");
        assert_eq!(v["reasons"][0], "low_memory");
        assert_eq!(v["reasons"][2], "prefers_reduced_motion");
    }
}

//! HCS VM configuration documents (pure JSON builders, tested everywhere).
//!
//! Shapes follow the HCS schema v2.1 field names (verified against
//! microsoft/hcsshim schema2 Go bindings): SchemaVersion, Owner,
//! VirtualMachine{Chipset{Uefi}, ComputeTopology{Memory,Processor},
//! Devices{Scsi}}.
//!
//! Secure Boot (§11): Debian ships a Microsoft-signed shim, so the Linux
//! path uses the `MicrosoftUEFICertificateAuthority` template — the CA
//! Hyper-V offers for non-Windows guests — never the Windows-only
//! template, and never silently disabled. If firmware rejects it on real
//! hardware, the documented fallback is `secure_boot: false` (recorded,
//! explicit, not silent).

use serde_json::{json, Value};

/// Minimal Gen2 Linux VM: 2 vCPU / 1536 MB, one SCSI-attached VHDX, no
/// network adapters, no shared folders, no clipboard, no video.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmSpec {
    pub disk_vhdx_path: String,
    pub vcpus: u8,
    pub memory_mb: u32,
    pub secure_boot: bool,
}

/// UEFI CA template trusted for distro shims (Hyper-V "Microsoft UEFI
/// Certificate Authority" option, NOT the Windows-only template).
pub const LINUX_SECURE_BOOT_TEMPLATE: &str = "MicrosoftUEFICertificateAuthority";

/// Build the HcsCreateComputeSystem configuration document.
/// `computer_id` becomes the HCS identity (stable Pegoles identity).
pub fn vm_config_json(computer_id: &str, spec: &VmSpec) -> String {
    let uefi = if spec.secure_boot {
        json!({ "SecureBootTemplateId": LINUX_SECURE_BOOT_TEMPLATE })
    } else {
        // Template absent == Secure Boot off. Explicit opt-out only;
        // callers must justify (documented fallback path).
        json!({})
    };
    serde_json::to_string(&json!({
        "Owner": "Pegoles",
        "SchemaVersion": { "Major": 2, "Minor": 1 },
        "ShouldTerminateOnLastHandleClosed": true,
        "VirtualMachine": {
            "StopOnReset": true,
            "Chipset": { "Uefi": uefi, "UseUtc": true },
            "ComputeTopology": {
                "Memory": { "SizeInMB": spec.memory_mb },
                "Processor": { "Count": spec.vcpus },
            },
            "Devices": {
                "Scsi": {
                    "0": {
                        "Attachments": {
                            "0": { "Path": spec.disk_vhdx_path, "Type": "VirtualDisk" },
                        },
                    },
                },
            },
        },
        "PegolesComputerId": computer_id,
    }))
    .expect("config JSON serializes")
}

/// Validate a config document structurally before handing it to HCS
/// (cheap fail-fast; HCS errors are expensive to debug).
pub fn validate_config_json(doc: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(doc).map_err(|e| format!("not JSON: {e}"))?;
    let vm = v.get("VirtualMachine").ok_or("missing VirtualMachine")?;
    vm.get("Chipset")
        .and_then(|c| c.get("Uefi"))
        .ok_or("missing Chipset.Uefi")?;
    let mem = vm
        .pointer("/ComputeTopology/Memory/SizeInMB")
        .and_then(|m| m.as_u64())
        .ok_or("missing ComputeTopology.Memory.SizeInMB")?;
    if !(512..=8192).contains(&mem) {
        return Err(format!("memory out of bounds: {mem}"));
    }
    let cpu = vm
        .pointer("/ComputeTopology/Processor/Count")
        .and_then(|c| c.as_u64())
        .ok_or("missing ComputeTopology.Processor.Count")?;
    if !(1..=8).contains(&cpu) {
        return Err(format!("vcpus out of bounds: {cpu}"));
    }
    let disk = vm
        .pointer("/Devices/Scsi/0/Attachments/0/Path")
        .and_then(|p| p.as_str())
        .ok_or("missing Devices.Scsi.0.Attachments.0.Path")?;
    if !disk.to_lowercase().ends_with(".vhdx") {
        return Err(format!("windows disk must be .vhdx, got: {disk}"));
    }
    let dtype = vm
        .pointer("/Devices/Scsi/0/Attachments/0/Type")
        .and_then(|t| t.as_str())
        .ok_or("missing attachment Type")?;
    if dtype != "VirtualDisk" {
        return Err(format!("attachment Type must be VirtualDisk, got: {dtype}"));
    }
    Ok(())
}

/// Pause options document (from the HCS Compute System Samples).
/// Live on Windows (pause path); kept compiled everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn pause_options_json() -> String {
    r#"{"SuspensionLevel":"Suspend"}"#.to_string()
}

/// Property query for VM power state (re-queried after every callback).
/// Live on Windows (state refresh); kept compiled everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn state_query_json() -> String {
    r#"{"PropertyTypes":["State"]}"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> VmSpec {
        VmSpec {
            disk_vhdx_path: r"C:\Pegoles\computers\abc\disk.vhdx".to_string(),
            vcpus: 2,
            memory_mb: 1536,
            secure_boot: true,
        }
    }

    #[test]
    fn config_has_required_schema_shape() {
        let doc = vm_config_json("computer-123", &spec());
        validate_config_json(&doc).expect("builder output must validate");
        let v: Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(v["Owner"], "Pegoles");
        assert_eq!(v["SchemaVersion"]["Major"], 2);
        assert_eq!(
            v["VirtualMachine"]["Chipset"]["Uefi"]["SecureBootTemplateId"],
            LINUX_SECURE_BOOT_TEMPLATE
        );
        assert_eq!(
            v["VirtualMachine"]["ComputeTopology"]["Memory"]["SizeInMB"],
            1536
        );
        assert_eq!(
            v["VirtualMachine"]["ComputeTopology"]["Processor"]["Count"],
            2
        );
        assert_eq!(v["ShouldTerminateOnLastHandleClosed"], true);
    }

    #[test]
    fn secure_boot_off_is_explicit_not_silent() {
        let off = VmSpec {
            secure_boot: false,
            ..spec()
        };
        let doc = vm_config_json("c", &off);
        let v: Value = serde_json::from_str(&doc).unwrap();
        assert!(v["VirtualMachine"]["Chipset"]["Uefi"]
            .get("SecureBootTemplateId")
            .is_none());
        // Still structurally valid (fallback path is explicit at call site).
        validate_config_json(&doc).expect("valid shape");
    }

    #[test]
    fn validator_rejects_non_vhdx_and_absurd_resources() {
        let mut bad = spec();
        bad.disk_vhdx_path = r"C:\x\disk.raw".to_string();
        assert!(validate_config_json(&vm_config_json("c", &bad)).is_err());
        let mut bad = spec();
        bad.memory_mb = 64;
        assert!(validate_config_json(&vm_config_json("c", &bad)).is_err());
        assert!(validate_config_json(r#"{"nope":1}"#).is_err());
    }

    #[test]
    fn pause_options_match_hcs_sample() {
        let v: Value = serde_json::from_str(&pause_options_json()).unwrap();
        assert_eq!(v["SuspensionLevel"], "Suspend");
    }
}

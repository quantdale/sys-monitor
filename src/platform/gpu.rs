// ---------------------------------------------------------------------------
// GPU CLASSIFICATION HELPERS — standalone functions used by the GPU pipeline
// ---------------------------------------------------------------------------

/// Classifies a GPU LUID as integrated or discrete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuClass {
    IGpu,
    DGpu,
    Unknown,
}

/// Extract the LUID hex string from a WMI GPUEngine `Name` field.
///
/// The WMI `Name` field can appear in two formats depending on the Windows version:
///   New (W10 20H2+): `"pid_1234_luid_0x00000000_0x00017D0F_phys_0_eng_0_engtype_3D"`
///   Old (legacy):    `"luid_0x00000000_0x00017D0F_phys_0_eng_0_engtype_3D"`
///
/// We want the *second* hex segment after `luid_` — e.g. `"0x00017D0F"`.
/// The first segment (`0x00000000`) is the high 32 bits of the LUID, which is
/// always zero on current Windows and carries no distinguishing information.
pub fn extract_luid_from_name(name: &str) -> Option<String> {
    // Try splitting on "_luid_" first (handles the pid_N_luid_... format).
    // If the name starts with "luid_" (no pid prefix), the split won't match
    // because there's no underscore before "luid". Handle that as a fallback.
    let after_luid = if let Some(pos) = name.find("_luid_") {
        // Skip past "_luid_" (6 chars)
        &name[pos + 6..]
    } else if name.starts_with("luid_") {
        // Legacy format: strip the "luid_" prefix (5 chars)
        &name[5..]
    } else {
        return None;
    };

    // after_luid is now "0x00000000_0x00017D0F_phys_..."
    // Split into at most 3 parts: ["0x00000000", "0x00017D0F", "phys_..."]
    let parts: Vec<&str> = after_luid.splitn(3, '_').collect();
    if parts.len() >= 2 && parts[1].starts_with("0x") {
        Some(parts[1].to_string())
    } else {
        None
    }
}

/// Classify a LUID as iGPU or dGPU.
///
/// **Primary:** keyword match on the vendor caption from `Win32_VideoController`.
/// This works on any machine where the positional LUID↔VideoController mapping is correct.
///
/// **Fallback:** hardcoded LUIDs known from the developer's machine. LUIDs are
/// assigned per-boot by Windows but tend to stay stable across reboots unless
/// the GPU driver is reinstalled. These values are machine-specific and serve as
/// a safety net when the vendor map is incomplete or misordered.
pub fn classify_luid(luid: &str, vendor_map: &std::collections::HashMap<String, String>) -> GpuClass {
    // Primary: vendor keyword matching (works on any machine)
    if let Some(vendor) = vendor_map.get(luid) {
        let v = vendor.to_lowercase();
        if v.contains("intel") {
            return GpuClass::IGpu;
        }
        if v.contains("nvidia") || v.contains("amd") || v.contains("radeon") {
            return GpuClass::DGpu;
        }
    }

    // Fallback: hardcoded LUIDs from the developer's machine (3-GPU system).
    // NOTE: These are machine-specific. On a different machine, unrecognised
    // LUIDs will fall through to GpuClass::Unknown and be logged.
    match luid {
        "0x00017A19" => GpuClass::IGpu,  // Intel iGPU (GDI Render, VideoProcessing engines)
        "0x00017C9F" => GpuClass::IGpu,  // Intel Xe display adapter (21× engtype_3D)
        "0x00017D0F" => GpuClass::DGpu,  // Nvidia dGPU (Cuda, VR, OFA_0, Compute engines)
        _ => GpuClass::Unknown,
    }
}

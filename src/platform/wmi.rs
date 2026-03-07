// ---------------------------------------------------------------------------
// GPU VENDOR MAP — Build a LUID-to-vendor-name map from Win32_VideoController
// ---------------------------------------------------------------------------
// Win32_VideoController gives adapter names (Intel, NVIDIA, AMD) but no LUIDs.
// Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine gives LUIDs but no names.
//
// We bridge the gap with a heuristic: collect unique LUIDs from the engine class,
// sort them alphabetically (Windows assigns LUIDs at PCI enumeration time, and the
// LUID hex values sort consistently with that order), then match positionally to
// VideoController entries in their own enumeration order.
//
// For the common laptop config (1 iGPU + 1 dGPU) and for desktops (1 GPU) this
// mapping is reliable. Systems with 3+ GPUs would need the D3DKMT API for an exact
// match, but that requires unsafe FFI — out of scope for this monitoring app.
pub fn build_gpu_vendor_map(
    wmi_con: &wmi::WMIConnection,
    gpu_debug: bool,
) -> std::collections::HashMap<String, String> {
    use std::collections::{HashMap, HashSet};

    // --- Step 1: collect ordered unique LUID prefixes from GPUEngine rows ---
    // Name format: "luid_0xHIGH_0xLOW_phys_P_eng_E_engtype_T"
    // We extract the LUID prefix: "luid_0xHIGH_0xLOW"
    //
    // We use HashMap<String, wmi::Variant> (instead of HashMap<String, String>)
    // because WMI fields can be integers, booleans, etc. — not all strings.
    // Deserializing into String when the wire type is an integer causes a
    // SerdeError. Variant holds any WMI type and lets us extract the value
    // via pattern matching on the concrete Variant arm.
    let luid_rows = match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
        "SELECT Name FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine",
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[GPU] build_gpu_vendor_map: LUID enumeration failed: {:?}", e);
            return HashMap::new();
        }
    };

    let mut luid_set: HashSet<String> = HashSet::new();
    for row in &luid_rows {
        if let Some(wmi::Variant::String(name)) = row.get("Name") {
            // Use the shared LUID extractor — handles both pid_N_luid_... and
            // bare luid_... Name formats. Returns e.g. "0x00017D0F".
            if let Some(luid) = crate::platform::gpu::extract_luid_from_name(name) {
                luid_set.insert(luid);
            }
        }
    }
    let mut luids: Vec<String> = luid_set.into_iter().collect();
    luids.sort(); // alphabetical sort of hex strings gives the same order as PCI enumeration

    // --- Step 2: enumerate VideoControllers for vendor names ---
    let vc_rows = match wmi_con.raw_query::<HashMap<String, wmi::Variant>>(
        "SELECT Caption FROM Win32_VideoController",
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[GPU] build_gpu_vendor_map: VideoController query failed: {:?}", e);
            return HashMap::new();
        }
    };

    // --- Step 3: match LUID[i] → VideoController[i] by index (positional heuristic) ---
    let mut map: HashMap<String, String> = HashMap::new();
    for (i, luid) in luids.iter().enumerate() {
        if let Some(vc) = vc_rows.get(i) {
            let caption = match vc.get("Caption") {
                Some(wmi::Variant::String(s)) => s.clone(),
                _ => String::new(),
            };
            map.insert(luid.clone(), caption);
        }
    }

    if gpu_debug {
        eprintln!("[GPU DEBUG] Vendor map: {:?}", map);
    }
    map
}

// ---------------------------------------------------------------------------
// GPU PERF COUNTERS — Query live 3D-engine utilization from GPUPerformanceCounters
// ---------------------------------------------------------------------------
// Returns Vec<(luid_prefix, util_pct)> — one entry per matching engine row.
//
// WHY GPUPerformanceCounters specifically:
//   Windows Task Manager's GPU tab reads
//   Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine — NOT the older
//   Gpu3d_Gpu3dEngine class. The GPUPerformanceCounters provider is registered
//   by display drivers on Windows 10+, and it is the authoritative source for
//   per-adapter, per-engine GPU utilization counters.
//
// WHAT IS A LUID:
//   Locally Unique Identifier — a 64-bit value Windows assigns to each GPU
//   adapter at runtime (renewed on every boot). It is Windows' canonical way
//   of referring to a GPU without relying on PCI bus addresses, which can
//   change if PCI enumeration order changes. The LUID appears in the Name
//   field as "pid_N_luid_0x<HIGH>_0x<LOW>_phys_P_eng_E_engtype_T".
//
// WHY WE SUM ACROSS ALL PIDs:
//   WMI returns ONE row per process per engine per GPU. Any single process
//   rarely shows significant utilization on its own. To get the total GPU
//   load (matching what Task Manager shows), we must SUM UtilizationPercentage
//   across all rows sharing the same LUID and engine type, regardless of PID.
//
//   Example:
//     pid_1234_luid_0x17D0F_engtype_3D → 2%
//     pid_5678_luid_0x17D0F_engtype_3D → 8%
//     pid_9012_luid_0x17D0F_engtype_3D → 15%
//                                      ──────
//     Total 3D utilization for 0x17D0F → 25%  ← this is what the graph shows
//
// WHY WE CAP AT 100%:
//   After summing, multi-engine GPUs can exceed 100% because the 3D engine may
//   be subdivided into multiple hardware queues. Capping at 100 keeps the graph
//   and percentage display in a sensible range.
//
// WHY WE ALSO TRACK VideoDecode:
//   Video decoding (H.264/HEVC/AV1) is a separate fixed-function engine.
//   It won't appear in engtype_3D numbers, but it's real GPU load.
//   We accumulate it separately for future UI display.
//
// Returns (3d_totals, video_totals):
//   3d_totals:    Vec<(luid, summed_3D_util%)>    — one entry per unique LUID
//   video_totals: Vec<(luid, summed_video_util%)>  — one entry per unique LUID
#[allow(dead_code)] // preserved WMI fallback; PDH is now the primary GPU source
pub fn query_gpu_perf_counters(
    wmi_con: &wmi::WMIConnection,
    gpu_debug: bool,
    gpu_error_logged: &mut bool,
) -> (Vec<(String, f64)>, Vec<(String, f64)>) {
    // Fetch ALL engine rows (no WHERE filter) so we can accumulate both
    // 3D and VideoDecode engines in a single pass.
    let query = "SELECT Name, UtilizationPercentage \
                 FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine";

    let rows = match wmi_con.raw_query::<std::collections::HashMap<String, wmi::Variant>>(query) {
        Ok(r) => r,
        Err(e) => {
            if !*gpu_error_logged {
                eprintln!("[GPU] WMI query failed: {:?}", e);
                eprintln!("[GPU] GPUPerformanceCounters class not found. \
                           GPU drivers may not expose WMI performance counters \
                           on this system (virtual machine, old driver, or WDDM < 2.0).");
                *gpu_error_logged = true;
            }
            return (vec![], vec![]);
        }
    };

    if rows.is_empty() {
        if !*gpu_error_logged {
            eprintln!("[GPU] WMI query returned no results. \
                       Class may not exist on this Windows version.");
            *gpu_error_logged = true;
        }
        return (vec![], vec![]);
    }

    // Accumulate utilization per LUID across all PIDs.
    // Key:   LUID string (e.g. "0x00017D0F")
    // Value: summed utilization % across all processes for that engine type
    let mut luid_3d_totals: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut luid_video_totals: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for row in &rows {
        if gpu_debug {
            eprintln!("[GPU DEBUG] Row: {:?}", row);
        }

        let name = match row.get("Name") {
            Some(wmi::Variant::String(s)) => s.clone(),
            _ => continue,
        };

        // Determine which engine type this row represents.
        // We only care about 3D (shader workload) and VideoDecode (HW decode).
        let is_3d = name.contains("engtype_3D");
        let is_video = name.contains("engtype_VideoDecode");
        if !is_3d && !is_video {
            continue; // Skip Copy, Compute, VR, OFA, GDI Render, etc.
        }

        // Extract the LUID from the Name string using the shared helper.
        let luid = match crate::platform::gpu::extract_luid_from_name(&name) {
            Some(l) => l,
            None => continue,
        };

        // UtilizationPercentage is returned as an integer by WMI drivers (UI4/UI8).
        // We match all numeric Variant arms and cast to f64.  The String arm covers
        // the rare case where an older driver serialises it as a string.
        let util: f64 = match row.get("UtilizationPercentage") {
            Some(wmi::Variant::UI4(n))  => *n as f64,
            Some(wmi::Variant::UI8(n))  => *n as f64,
            Some(wmi::Variant::I4(n))   => *n as f64,
            Some(wmi::Variant::I8(n))   => *n as f64,
            Some(wmi::Variant::R8(n))   => *n,
            Some(wmi::Variant::R4(n))   => *n as f64,
            Some(wmi::Variant::String(s)) => s.parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        };

        // SUM across all PIDs for this LUID — this is the critical aggregation fix.
        // Before this fix, we were keeping individual per-row values and taking max(),
        // which showed near-zero because any single process rarely uses much GPU alone.
        if is_3d {
            *luid_3d_totals.entry(luid).or_insert(0.0) += util;
        } else {
            *luid_video_totals.entry(luid).or_insert(0.0) += util;
        }
    }

    // Cap each LUID total at 100% — summing across many processes and engine
    // instances can exceed 100 on multi-engine GPUs.
    let capped_3d: Vec<(String, f64)> = luid_3d_totals
        .into_iter()
        .map(|(luid, total)| (luid, total.min(100.0)))
        .collect();

    let capped_video: Vec<(String, f64)> = luid_video_totals
        .into_iter()
        .map(|(luid, total)| (luid, total.min(100.0)))
        .collect();

    if gpu_debug {
        eprintln!("[GPU DEBUG] 3D totals (summed, capped): {:?}", capped_3d);
        eprintln!("[GPU DEBUG] Video totals (summed, capped): {:?}", capped_video);
    }

    (capped_3d, capped_video)
}

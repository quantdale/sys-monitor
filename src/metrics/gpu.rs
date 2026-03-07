use windows::Win32::System::Performance::{
    PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
};

// ---------------------------------------------------------------------------
// GPU UTILIZATION — PDH-based entry point: returns (igpu_util%, dgpu_util%)
// ---------------------------------------------------------------------------
// Uses PDH (Performance Data Helper) directly — the same API Windows Task
// Manager uses for its GPU graphs. PDH gives more accurate readings than WMI
// because it manages its own per-query-handle baseline for rate computation.
//
// WHY PDH INSTEAD OF WMI:
//   Task Manager uses PDH directly. WMI's Win32_PerfFormattedData classes are
//   a higher-level wrapper that queries PDH internally, adding an abstraction
//   layer that introduces inaccuracy. For performance counters, PDH is always
//   the authoritative source.
//
// PDH LIFECYCLE SUMMARY (see struct fields for full explanation):
//   PdhOpenQuery (once) → PdhAddEnglishCounter (once) → every poll:
//     PdhCollectQueryData (in refresh_metrics) → read arrays for disk/GPU
//
// WHY unsafe:
//   PDH is a C Win32 API. Rust's `unsafe` block is required at any FFI
//   (Foreign Function Interface) boundary — calling native C code from Rust.
//   `unsafe` means "I take responsibility for correctness here" (pointer
//   validity, alignment, lifetime). It does NOT mean the code is wrong.
//
// WHY FIRST POLL RETURNS 0%:
//   PDH computes utilization as: (value₂ − value₁) / time_delta.
//   The first PdhCollectQueryData (called in new_pdh_gpu_query at startup)
//   establishes value₁ (the baseline). The next poll computes the first real
//   delta. Identical to Task Manager showing 0 on first second.
pub fn query_gpu_utilization_pdh(app: &mut crate::app::SystemMonitor) -> (f64, f64) {
    if app.pdh_query.is_none() {
        return (0.0, 0.0); // PDH init failed at startup
    }
    let counter_3d = match app.pdh_gpu_3d_counter {
        Some(c) => c,
        None => return (0.0, 0.0),
    };

    // Build the LUID → vendor name map for iGPU/dGPU classification.
    // Uses the take/put-back pattern on app.wmi_con so we can call a
    // &mut app method (build_gpu_vendor_map) while wmi_con is live.
    // WMI is only used here for static Win32_VideoController name lookup.
    let vendor_map = match app.wmi_con.take() {
        Some(con) => {
            let map = crate::platform::wmi::build_gpu_vendor_map(&con, app.gpu_debug);
            app.wmi_con = Some(con);
            map
        }
        None => std::collections::HashMap::new(),
    };

    let mut luid_3d_totals: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();

    // SAFETY: All Win32 PDH function calls via FFI.
    //   • PDH_HQUERY / PDH_HCOUNTER are opaque scalar handles (safe to copy)
    //   • Mutable pointer arguments (&mut buf_size, etc.) point to stack variables
    //   • The backing buffer is heap-allocated with u64 alignment (8 bytes),
    //     sufficient for PDH_FMT_COUNTERVALUE_ITEM_W which contains an f64 union
    //   • We verify return codes before reading any output data
    //   • szName pointers in PDH_FMT_COUNTERVALUE_ITEM_W point into the same
    //     backing buffer, which stays alive for the duration of this unsafe block
    unsafe {
        // --- Probe call: determine the required buffer size ---
        // PdhGetFormattedCounterArrayW with a null instance buffer returns
        // PDH_MORE_DATA (a "failure") but populates buf_size and item_count.
        // We intentionally ignore the return value of this probe call.
        //
        // PDH_FMT_DOUBLE (0x200) requests values as f64 in the 0.0–100.0 range.
        let mut buf_size: u32 = 0;
        let mut item_count: u32 = 0;
        let _ = PdhGetFormattedCounterArrayW(
            counter_3d,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            None,
        );

        if item_count == 0 {
            // No GPU Engine instances matched the wildcard.
            // Expected on the first poll (no baseline yet), or on systems
            // without GPU hardware counters (VMs, old drivers). Return 0% quietly.
            return (0.0, 0.0);
        }

        // --- Allocate backing buffer with guaranteed 8-byte alignment ---
        // PDH_FMT_COUNTERVALUE_ITEM_W contains a union field with an f64 member,
        // which requires 8-byte alignment. Vec<u8> only guarantees 1-byte alignment.
        // Vec<u64> guarantees 8-byte alignment and is safe to reinterpret as the
        // target struct because:
        //   • We only READ through the typed pointer (no aliased writes)
        //   • Both types are POD (no Drop, no invalid bit patterns)
        //   • We allocate enough bytes (with a 3× safety margin for new processes
        //     that may have started between the probe call and this data call)
        let u64_count = (buf_size as usize * 3 + 7) / 8;
        let mut backing: Vec<u64> = vec![0u64; u64_count];
        let mut actual_buf_size: u32 = (u64_count * 8) as u32;
        let buf_ptr = backing.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;

        // --- Data call: fill buffer with one entry per matched GPU engine instance ---
        // Instance name format (same as WMI Name field):
        //   "pid_1234_luid_0x00000000_0x00017D0F_phys_0_eng_0_engtype_3D"
        // We use extract_luid_from_name() to get the LUID from each instance.
        let status = PdhGetFormattedCounterArrayW(
            counter_3d,
            PDH_FMT_DOUBLE,
            &mut actual_buf_size,
            &mut item_count,
            Some(buf_ptr),
        );

        if status != 0 {
            // PDH_CSTATUS_INVALID_DATA is expected on the very first call
            // (no baseline yet). Subsequent calls return valid data.
            return (0.0, 0.0);
        }

        // Iterate over instances, extract LUID, accumulate utilization.
        for i in 0..item_count as usize {
            let item: &PDH_FMT_COUNTERVALUE_ITEM_W = &*buf_ptr.add(i);

            // PDH_CSTATUS_VALID_DATA = 0x0, PDH_CSTATUS_NEW_DATA = 0x1.
            // Any other status means this instance has no valid data this cycle.
            if item.FmtValue.CStatus > 1 {
                continue;
            }

            // Convert PWSTR instance name to a Rust String.
            // szName points into the same backing buffer allocation, which is
            // kept alive for the duration of this unsafe block.
            let name = match item.szName.to_string() {
                Ok(s) => s,
                Err(_) => continue,
            };

            if app.gpu_debug {
                eprintln!("[PDH DEBUG] instance: {}", name);
            }

            // Extract LUID using the same helper as the WMI pipeline.
            // PDH and WMI share the same counter infrastructure and the same
            // instance name format, so this helper works for both.
            let luid = match crate::platform::gpu::extract_luid_from_name(&name) {
                Some(l) => l,
                None => continue,
            };

            // doubleValue is valid because we requested PDH_FMT_DOUBLE above.
            // Accessing the union requires unsafe — we are already in an unsafe block.
            let util = item.FmtValue.Anonymous.doubleValue.clamp(0.0, 100.0);

            // SUM utilization across all processes for this LUID.
            // Each instance is one process's contribution to the total GPU load.
            *luid_3d_totals.entry(luid).or_insert(0.0) += util;
        }
    }

    // Classify each LUID as iGPU or dGPU and take the max per class.
    // Capping at 100% handles multi-engine GPUs where summing can exceed 100.
    let mut igpu_max = 0.0f64;
    let mut dgpu_max = 0.0f64;

    for (luid, total) in luid_3d_totals {
        let capped = total.min(100.0);
        match crate::platform::gpu::classify_luid(&luid, &vendor_map) {
            crate::platform::gpu::GpuClass::IGpu => igpu_max = igpu_max.max(capped),
            crate::platform::gpu::GpuClass::DGpu => dgpu_max = dgpu_max.max(capped),
            crate::platform::gpu::GpuClass::Unknown => {
                if !app.gpu_error_logged {
                    eprintln!("[PDH] Unclassified LUID: {} (util={:.1}%)", luid, capped);
                }
            }
        }
    }

    if app.gpu_debug {
        eprintln!(
            "[PDH DEBUG] Final: igpu_max={:.1}%, dgpu_max={:.1}%",
            igpu_max, dgpu_max
        );
    }

    (igpu_max, dgpu_max)
}

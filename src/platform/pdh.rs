// ---------------------------------------------------------------------------
// PDH IMPORTS
// ---------------------------------------------------------------------------
// PDH = Performance Data Helper — the Win32 API Windows uses internally for all
// hardware performance counters, including the GPU engine counters shown in
// Task Manager. Using PDH directly (instead of WMI's PerfFormattedData wrapper)
// gives us accurate baseline tracking and the same readings as Task Manager:
//
//   Task Manager:   PDH directly → GPU counter → accurate %
//   Old WMI path:   WMI → PerfFormattedData → PDH internally → less accurate %
//
// The `windows` crate is Microsoft's official Rust bindings to the Win32 API.
// It is maintained by Microsoft and already used internally by eframe.
// We prefer it over third-party PDH crates for authoritativeness and version alignment.
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData,
    PdhOpenQueryW,
};

// ---------------------------------------------------------------------------
// PDH INITIALIZATION HELPER
// ---------------------------------------------------------------------------

/// Open a PDH query and register GPU + disk utilization counters once at startup.
///
/// Returns `Some((query, counter_3d, counter_video_opt, counter_disk_opt))` on success.
/// Returns `None` if the query or 3D counter cannot be opened (GPU tracking disabled).
/// `counter_video_opt` is `None` if the VideoDecode counter is unavailable (non-fatal).
/// `counter_disk_opt` is `None` if % Disk Time is unavailable (non-fatal).
///
/// **Why open once:**
///   PDH rate-based counters (Win32_PerfFormattedData_* counters) track utilization as
///   a delta between two `PdhCollectQueryData` calls: `(value₂ − value₁) / time_delta`.
///   This baseline is stored INSIDE the query handle. Recreating the handle on every
///   poll discards the baseline and always returns 0% — the identical bug that existed
///   in the previous WMI approach.
///
/// **`unsafe` explanation:**
///   PDH is a Win32 C API. `unsafe` is required for any FFI (Foreign Function Interface)
///   call to C code because the Rust compiler cannot verify the safety of foreign code.
///   Here we ensure safety manually: all pointer arguments are valid stack variables or
///   properly aligned heap allocations, and we check all return codes.
pub fn new_pdh_gpu_query() -> Option<(isize, isize, Option<isize>, Option<isize>)> {
    // SAFETY: PDH C API calls via FFI. All mutable pointer arguments are stack variables.
    // Return codes are checked before any output values are read.
    unsafe {
        let mut query: isize = 0;

        // PdhOpenQueryW: creates the query container.
        //   • First arg (None): live system data, not a log file
        //   • Second arg (0): no callback userdata needed
        //   • Third arg: receives the allocated query handle
        if PdhOpenQueryW(None, 0, &mut query) != 0 {
            eprintln!("[PDH] PdhOpenQueryW failed — GPU metrics unavailable.");
            return None;
        }

        // PdhAddEnglishCounterW: registers a counter path using English names,
        // regardless of the system's display locale. This ensures the path
        // `\GPU Engine(*engtype_3D*)\Utilization Percentage` resolves correctly
        // on French, German, Chinese, etc. Windows installations.
        //
        // Counter path format:  \ObjectName(InstanceFilter)\CounterName
        //   \GPU Engine          — the PDH performance object (driver-registered)
        //   (*engtype_3D*)       — wildcard instance filter: all 3D engine instances
        //                          across all processes and all GPU adapters
        //   \Utilization Percentage — the specific counter within that object
        //
        // The wildcard (*) is resolved at PdhCollectQueryData time, not here.
        // Every process with a loaded 3D engine will produce a separate instance.
        let path_3d =
            windows::core::w!(r"\GPU Engine(*engtype_3D*)\Utilization Percentage");
        let mut counter_3d: isize = 0;
        if PdhAddEnglishCounterW(query, path_3d, 0, &mut counter_3d) != 0 {
            eprintln!("[PDH] Failed to add GPU 3D counter — GPU metrics unavailable.");
            return None;
        }

        // VideoDecode counter — hardware video decode (H.264/HEVC/AV1) load.
        // This is a separate fixed-function engine; video playback won't appear
        // in 3D % numbers. Non-fatal: if unavailable we skip it and continue.
        let path_video = windows::core::w!(
            r"\GPU Engine(*engtype_VideoDecode*)\Utilization Percentage"
        );
        let mut counter_video: isize = 0;
        let counter_video_opt =
            if PdhAddEnglishCounterW(query, path_video, 0, &mut counter_video) == 0 {
                Some(counter_video)
            } else {
                eprintln!("[PDH] VideoDecode counter unavailable — video GPU tracking disabled.");
                None
            };

        // Disk % Idle Time counter. Added to the SAME query as GPU so one
        // PdhCollectQueryData call snapshots both domains atomically and reuses
        // the same baseline state. Opening a second query would add drift and
        // duplicate rate baselines unnecessarily.
        //
        // We use % Idle Time rather than % Disk Time / % Disk Active Time because
        // % Idle Time is reliably present on all Windows 10/11 configurations while
        // % Disk Active Time can be absent on some storage drivers. Active time is
        // computed as: active% = 100 - idle%.  Values are inverted in query_disk_active_time.
        //
        // PhysicalDisk instance names look like:
        //   "0 C: D:"  (disk index + one/more mounted drive letters)
        //   "_Total"    (aggregate over all physical disks)
        // We ignore _Total in UI and render per-disk cards instead.
        let path_disk_active = windows::core::w!(r"\PhysicalDisk(*)\% Idle Time");
        let mut counter_disk_active: isize = 0;
        let counter_disk_opt =
            if PdhAddEnglishCounterW(query, path_disk_active, 0, &mut counter_disk_active) == 0 {
                Some(counter_disk_active)
            } else {
                eprintln!("[PDH] Failed to add disk idle time counter.");
                None
            };

        // First PdhCollectQueryData — establishes the sample baseline.
        // This call stores value₁ for every matched instance. The NEXT call
        // (first actual poll ~1 second after app start) will compute value₂ − value₁
        // and return real utilization percentages.
        // The first call always "returns" 0% — this is correct, not a bug.
        let _ = PdhCollectQueryData(query);

        eprintln!("[PDH] GPU/disk counters initialized successfully.");
        Some((query, counter_3d, counter_video_opt, counter_disk_opt))
    }
}

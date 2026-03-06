// =============================================================================
// HOW TO SET UP AND RUN ON WINDOWS
//
// Prerequisites:
//   Verify Rust is installed: rustc --version
//   If not installed: https://rustup.rs
//
// You may need the Visual Studio C++ Build Tools for Rust to compile on Windows:
//   Download: https://visualstudio.microsoft.com/visual-cpp-build-tools/
//   During install, check: "Desktop development with C++"
//   This gives Rust's compiler the Windows linker it needs.
//   (You do NOT need to write any C++ — this is just a build tool dependency)
//
// Run the app (debug build — fast to compile, slower to run):
//   cd sys-monitor
//   cargo run
//
// Optimized release build — smaller, faster .exe:
//   cargo build --release
//
// The .exe location after release build:
//   sys-monitor/target/release/sys-monitor.exe
//   This file is standalone — double-click it on any Windows machine, no install needed.
// =============================================================================

// ---------------------------------------------------------------------------
// WINDOWS SUBSYSTEM ATTRIBUTE
// ---------------------------------------------------------------------------
// This tells the Windows linker to create a GUI application, not a console app.
// Without it, Windows would open a black cmd.exe terminal behind your window.
// Equivalent to: Project Properties → Linker → System → Subsystem: Windows in MSVC.
// We only apply it on release builds so we can still see panic messages during
// development (cargo run keeps the console open in debug mode).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ---------------------------------------------------------------------------
// IMPORTS
// ---------------------------------------------------------------------------
// In Rust, `use` is like `import` in JavaScript/TypeScript.
// We bring names into scope so we don't have to type the full path every time.
use eframe::egui;                        // The immediate-mode GUI widgets
use egui::Color32;                       // RGB color type
use egui_plot::{Line, Plot, PlotPoints}; // The graphing widgets
use std::collections::VecDeque;          // Double-ended queue — explained below
use std::time::{Duration, Instant};      // Monotonic clock, like performance.now() in JS
use sysinfo::System;                     // Wraps Windows system APIs
// Disks is a separate handle from System in sysinfo 0.30. The library splits
// disk stats out so you only pay the cost of querying them when you ask.
// Under the hood on Windows this calls DeviceIoControl with IOCTL_DISK_PERFORMANCE
// to read per-disk I/O counters — the same source Task Manager uses.
use sysinfo::Disks;
// Networks is sysinfo's handle for querying network interface statistics.
// Under the hood on Windows, sysinfo calls GetIfTable2() / GetIfEntry2()
// from the IP Helper API (iphlpapi.dll) — the same source as Resource Monitor.
// It exposes per-interface counters that update every time you call refresh().
// Unlike the disk API (which returns cumulative bytes-since-boot), the
// Networks API returns the DELTA bytes since the LAST refresh(), so we don't
// need to manually subtract a previous snapshot — sysinfo does it for us.
use sysinfo::Networks;

// ---------------------------------------------------------------------------
// WHAT IS A VecDeque?
// ---------------------------------------------------------------------------
// VecDeque = "Vector Double-Ended Queue". Think of it like an array (Vec) but
// with O(1) push AND pop from BOTH ends (front and back), not just the back.
//
// Why use it here instead of a plain Vec?
//   - Our history buffer is a sliding window: we add new data at the BACK and
//     remove old data from the FRONT (the oldest value).
//   - With a plain Vec, removing from the front shifts every element left → O(n).
//   - With VecDeque, the front pointer just advances → O(1). Much more efficient.
//
// Web analogy: Imagine an array where you do `arr.push(newValue)` every second
// and `arr.shift()` to remove the oldest. In JavaScript that's fine for 60 items,
// but in a hot loop with thousands of items, VecDeque is the professional choice.
//
// VecDeque is backed by a ring buffer internally — picture a circular array where
// the "start" and "end" pointers wrap around instead of the data moving.

// ---------------------------------------------------------------------------
// APP STATE STRUCT
// ---------------------------------------------------------------------------
// In egui's immediate-mode model, YOU own all state. There is no hidden DOM
// or component tree storing values. You define a struct that holds everything
// your app needs between frames, and you pass it to egui each frame.
//
// Web analogy: This is like your React component's `useState` or a Svelte store —
// it's the single source of truth for everything the UI will display.
struct SystemMonitor {
    // sysinfo::System is the main entry point to the sysinfo crate.
    // Under the hood on Windows it:
    //   - For CPU: opens PDH (Performance Data Helper) counters, specifically
    //     the "\Processor(_Total)\% Processor Time" counter. This is the same
    //     counter that Task Manager reads. PDH is a Win32 subsystem.
    //   - For Memory: calls GlobalMemoryStatusEx(), a Win32 API function that
    //     returns a MEMORYSTATUSEX struct with dwTotalPhys and dwAvailPhys fields.
    //     Used Memory = Total - Available.
    // sysinfo wraps all this Win32 complexity into clean, safe Rust.
    system: System,

    // Sliding window of the last N CPU usage samples (0.0 – 100.0).
    // We store f64 (64-bit float) because egui_plot expects f64 coordinates.
    cpu_history: VecDeque<f64>,

    // Same sliding window for memory usage, stored as a percentage (0.0 – 100.0)
    // so both graphs share the same Y-axis scale.
    mem_history: VecDeque<f64>,

    // std::time::Instant is a monotonic clock timestamp — it only goes forward,
    // immune to system clock changes (unlike wall-clock time).
    // Web analogy: performance.now() returns milliseconds since page load.
    //              Instant::now() returns an opaque point in time.
    //              elapsed() gives you the Duration since that point.
    // We use this to throttle our system polling to once per second.
    last_update: Instant,

    // Number of data points to display in the current time window.
    // Dynamically set by the time range selector (30, 60, 300, 600, 1800, 3600).
    history_length: usize,

    // The maximum capacity of the history buffer — always 3600 (one data point
    // per second for 1 full hour). The buffer must be this large regardless of
    // the user's currently selected time range, because the user might switch
    // from "30s" to "1h" at any time and expects to see all data that was
    // silently collected in the background. If the buffer were capped at the
    // display window size, switching to a longer range would show gaps.
    max_history: usize,

    // The currently selected duration in seconds, controlled by the header
    // buttons (30, 60, 300, 600, 1800, 3600). Default: 60 (1 minute).
    selected_duration: u64,

    // ── DISK I/O ────────────────────────────────────────────────────────────
    // sysinfo::Disks is a dedicated handle for querying disk statistics.
    // It is separate from System because disk I/O polling is relatively expensive —
    // separating it lets you refresh disks independently of CPU/RAM.
    disks: Disks,

    // Read and write rate histories for C: and D: drives, stored in MB/s.
    // We track read and write separately so both lines can appear on the same graph,
    // just like Task Manager shows two lines per drive in its Disk section.
    disk_c_read_history:  VecDeque<f64>, // C: drive read  MB/s history
    disk_c_write_history: VecDeque<f64>, // C: drive write MB/s history
    disk_d_read_history:  VecDeque<f64>, // D: drive read  MB/s history
    disk_d_write_history: VecDeque<f64>, // D: drive write MB/s history

    // Previous cumulative byte counters for each drive.
    //
    // sysinfo reports TOTAL bytes read/written since boot (a monotonically
    // increasing counter — like a car odometer). To convert that to a RATE
    // (MB per second), we subtract the previous snapshot from the current one:
    //     rate_MB_s = (current_bytes - prev_bytes) / elapsed_seconds / 1024² 
    //
    // This is the same delta technique used by all system monitors:
    // Task Manager, Resource Monitor, and perfmon all compute rates this way.
    disk_c_prev_read:  u64,
    disk_c_prev_write: u64,
    disk_d_prev_read:  u64,
    disk_d_prev_write: u64,

    // ── NETWORK I/O ──────────────────────────────────────────────────────────
    // Networks is a HashMap<String, NetworkData> under the hood — each key is
    // the OS-assigned interface name (e.g. "Ethernet", "Wi-Fi", "vEthernet").
    // We keep one handle and call refresh() on it every second.
    networks: Networks,

    // Combined download (received) and upload (transmitted) rate across all
    // non-loopback interfaces, stored in KB/s.
    // Aggregating all adapters matches Task Manager's "Total" network view.
    // We use KB/s rather than MB/s for network because most traffic is in the
    // tens-of-KB/s range; MB/s would make the graph flat most of the time.
    net_recv_history: VecDeque<f64>, // download KB/s history
    net_sent_history: VecDeque<f64>, // upload   KB/s history

    // ── GPU UTILIZATION ─────────────────────────────────────────────────────────
    // We track iGPU (Intel integrated) and dGPU (discrete: NVIDIA/AMD) separately.
    // On Windows, we query WMI for GPU Engine utilization percentage via the
    // Win32_PerfFormattedData_Gpu3d_Gpu3dEngine class. iGPU typically reports
    // as "Intel(R) UHD Graphics" or similar; dGPU as "NVIDIA GeForce RTX" etc.
    //
    // If a GPU is not present, its history remains empty (displays 0%).
    // Utilization is stored as a percentage (0.0 – 100.0).
    igpu_history: VecDeque<f64>, // Intel iGPU utilization % history
    dgpu_history: VecDeque<f64>, // Discrete GPU (NVIDIA/AMD) utilization % history
}

impl SystemMonitor {
    // `new()` is Rust's convention for a constructor. There's no `new` keyword —
    // it's just a static method that returns Self (the type being implemented).
    // Web analogy: This is like a class constructor in TypeScript:
    //   constructor() { this.system = new System(); this.cpuHistory = []; ... }
    fn new() -> Self {
        // Tell sysinfo which components we intend to use.
        // This avoids loading kernel modules we don't need, saving startup time.
        // REFRESH_CPU_ALL: enables CPU usage tracking
        // REFRESH_MEMORY:  enables RAM tracking
        let mut system = System::new_with_specifics(
            sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::everything())
                .with_memory(sysinfo::MemoryRefreshKind::everything()),
        );

        // sysinfo requires an initial refresh + a short sleep before the first
        // CPU reading is meaningful. CPU usage is calculated as a DELTA between
        // two snapshots (like Task Manager does). Without a prior snapshot,
        // the first reading would be 0 or garbage.
        // This mirrors how PDH counters work: you must call PdhCollectQueryData
        // twice with a gap between them to get a rate-based counter value.
        system.refresh_all();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        system.refresh_all();

        // Initialise the disk handle and do a first refresh so we have a baseline
        // byte counter to subtract from on the very next 1-second tick.
        // new_with_refreshed_list() calls DeviceIoControl for each disk immediately.
        let mut disks = Disks::new_with_refreshed_list();
        disks.refresh(false); // second pass to seed the I/O counters

        // Seed the "previous" counters from the initial read so the first delta
        // on the first 1-second tick is meaningful rather than equal to bytes-since-boot.
        let (c_r0, c_w0, d_r0, d_w0) = Self::sample_disk_bytes(&disks);

        // Initialise the Networks handle.
        // new_with_refreshed_list() calls GetIfTable2() immediately to discover all
        // interfaces and seed the internal counters, so the first refresh() call
        // will already produce a valid delta (not bytes-since-boot).
        let mut networks = Networks::new_with_refreshed_list();
        // A second refresh seeds the per-interface delta counters so the first
        // reading we display is sensible rather than a large spike from boot stats.
        networks.refresh(false);

        SystemMonitor {
            system,
            // Pre-allocate VecDeques for the FULL 1-hour buffer (3600 data points).
            // Even though we default to showing only 60 seconds, we always store
            // up to 3600 points so the user can switch to a longer range at any
            // time without losing data that was collected in the background.
            cpu_history: VecDeque::with_capacity(3600),
            mem_history: VecDeque::with_capacity(3600),
            // Instant::now() captures "right now". We subtract a full second so
            // the very first frame immediately triggers a data refresh instead of
            // waiting one second for the first reading.
            last_update: Instant::now() - Duration::from_secs(1),
            history_length: 60,       // default display window: 1 minute
            max_history: 3600,        // buffer cap: 1 hour (3600 seconds)
            selected_duration: 60,    // default selected time range: 1 minute
            disks,
            disk_c_read_history:  VecDeque::with_capacity(3600),
            disk_c_write_history: VecDeque::with_capacity(3600),
            disk_d_read_history:  VecDeque::with_capacity(3600),
            disk_d_write_history: VecDeque::with_capacity(3600),
            disk_c_prev_read:  c_r0,
            disk_c_prev_write: c_w0,
            disk_d_prev_read:  d_r0,
            disk_d_prev_write: d_w0,
            networks,
            net_recv_history: VecDeque::with_capacity(3600),
            net_sent_history: VecDeque::with_capacity(3600),
            igpu_history: VecDeque::with_capacity(3600),
            dgpu_history: VecDeque::with_capacity(3600),
        }
    }

    // ---------------------------------------------------------------------------
    // HELPER: extract CUMULATIVE byte counters for C: and D: from the Disks list.
    // Returns (c_read_total, c_write_total, d_read_total, d_write_total) in bytes.
    //
    // We use total_read_bytes() / total_written_bytes() (bytes since boot) rather
    // than read_bytes() / written_bytes() (bytes since last refresh) so that our
    // explicit delta calculation is the one source of truth for the rate.
    // This matches how Task Manager and perfmon compute disk throughput.
    //
    // mount_point() on Windows returns a Path like "C:\" or "D:\".  We call
    // to_string_lossy() to get a &str and check the drive letter prefix.
    // ---------------------------------------------------------------------------
    fn sample_disk_bytes(disks: &Disks) -> (u64, u64, u64, u64) {
        let (mut c_r, mut c_w) = (0u64, 0u64);
        let (mut d_r, mut d_w) = (0u64, 0u64);
        for disk in disks.list() {
            let mp = disk.mount_point().to_string_lossy().to_uppercase();
            if mp.starts_with("C:") {
                c_r = disk.usage().total_read_bytes;
                c_w = disk.usage().total_written_bytes;
            } else if mp.starts_with("D:") {
                d_r = disk.usage().total_read_bytes;
                d_w = disk.usage().total_written_bytes;
            }
        }
        (c_r, c_w, d_r, d_w)
    }

    // Polls the OS for fresh CPU and memory data and pushes it into our history.
    // This is only called once per second (throttled by last_update check in update()).
    fn refresh_metrics(&mut self) {
        // Refresh CPU counters. sysinfo re-queries the PDH counter and computes
        // (new_idle_time - old_idle_time) / elapsed to get a CPU usage percentage.
        self.system.refresh_cpu_usage();

        // Refresh memory counters. sysinfo calls GlobalMemoryStatusEx() again.
        self.system.refresh_memory();

        // global_cpu_usage() returns a f32 representing the average usage % across ALL
        // logical cores. In sysinfo 0.33+, this is a direct f32 value (no .cpu_usage() call).
        // Example: if you have 8 cores and average utilization is 25%, this returns 25.0.
        let cpu_pct = self.system.global_cpu_usage() as f64;

        // used_memory() / total_memory() both return kilobytes (KB) as u64.
        // We convert to a percentage so the graph Y-axis matches the CPU graph (0–100).
        let used_mem_kb = self.system.used_memory();
        let total_mem_kb = self.system.total_memory();
        let mem_pct = if total_mem_kb > 0 {
            (used_mem_kb as f64 / total_mem_kb as f64) * 100.0
        } else {
            0.0
        };

        // Push new values to the BACK of the deque (most recent end).
        self.cpu_history.push_back(cpu_pct);
        self.mem_history.push_back(mem_pct);

        // Pop from the front when the buffer exceeds max_history (3600), NOT
        // history_length. We always retain the full hour of data so the user
        // can freely switch between time ranges without losing history.
        if self.cpu_history.len() > self.max_history {
            self.cpu_history.pop_front();
        }
        if self.mem_history.len() > self.max_history {
            self.mem_history.pop_front();
        }

        // ── DISK I/O REFRESH ────────────────────────────────────────────────
        // disks.refresh(false) calls DeviceIoControl(IOCTL_DISK_PERFORMANCE) for each
        // disk and updates the cumulative read_bytes / written_bytes counters.
        // The bool arg: false = don't remove disks that have disappeared (safe for most cases).
        self.disks.refresh(false);

        // Read the new cumulative counters.
        let (c_r, c_w, d_r, d_w) = Self::sample_disk_bytes(&self.disks);

        // Compute deltas: bytes transferred since the last 1-second tick.
        // saturating_sub prevents underflow if a counter ever wraps or resets
        // (rare but possible after system sleep/resume on some drivers).
        // Divide by 1024² to convert bytes → megabytes (MB/s at 1 Hz polling).
        let c_read_mbs  = c_r.saturating_sub(self.disk_c_prev_read)  as f64 / (1024.0 * 1024.0);
        let c_write_mbs = c_w.saturating_sub(self.disk_c_prev_write) as f64 / (1024.0 * 1024.0);
        let d_read_mbs  = d_r.saturating_sub(self.disk_d_prev_read)  as f64 / (1024.0 * 1024.0);
        let d_write_mbs = d_w.saturating_sub(self.disk_d_prev_write) as f64 / (1024.0 * 1024.0);

        // Store current counters as the baseline for the next tick.
        self.disk_c_prev_read  = c_r;
        self.disk_c_prev_write = c_w;
        self.disk_d_prev_read  = d_r;
        self.disk_d_prev_write = d_w;

        // Push rates into the sliding-window histories (capped at max_history, not history_length).
        Self::push_history(&mut self.disk_c_read_history,  c_read_mbs,  self.max_history);
        Self::push_history(&mut self.disk_c_write_history, c_write_mbs, self.max_history);
        Self::push_history(&mut self.disk_d_read_history,  d_read_mbs,  self.max_history);
        Self::push_history(&mut self.disk_d_write_history, d_write_mbs, self.max_history);

        // ── NETWORK I/O REFRESH ─────────────────────────────────────────────
        // networks.refresh(false) calls GetIfEntry2() for each interface.
        // After this call, received() and transmitted() on each NetworkData
        // return the bytes transferred since the PREVIOUS refresh() — i.e. the
        // delta is pre-computed by sysinfo. At 1 Hz polling, delta == bytes/sec.
        self.networks.refresh(false);

        // Accumulate bytes across all interfaces, skipping:
        //   - Loopback ("lo" on Linux, "Loopback*" on Windows) — localhost traffic
        //     would massively inflate the reading with no useful signal.
        //   - Interfaces with zero traffic this tick — doesn't change the sum but
        //     avoids counting inactive virtual adapters (e.g. VirtualBox, WSL).
        //
        // "Loopback" check: Windows names loopback as "Loopback Pseudo-Interface N"
        // and Linux as "lo". We filter both with a case-insensitive prefix check.
        let mut total_recv_bytes = 0u64;
        let mut total_sent_bytes = 0u64;
        for (iface_name, data) in &self.networks {
            let name_upper = iface_name.to_uppercase();
            if name_upper.contains("LOOPBACK") || name_upper == "LO" {
                continue; // skip loopback
            }
            total_recv_bytes += data.received();
            total_sent_bytes += data.transmitted();
        }

        // Convert bytes/sec → KB/s (divide by 1024).
        // We use KB/s for network because typical usage (streaming, browsing)
        // sits in the 100–10 000 KB/s range — MB/s would make the graph too flat.
        let recv_kbs = total_recv_bytes as f64 / 1024.0;
        let sent_kbs = total_sent_bytes as f64 / 1024.0;

        Self::push_history(&mut self.net_recv_history, recv_kbs, self.max_history);
        Self::push_history(&mut self.net_sent_history, sent_kbs, self.max_history);

        // ── GPU UTILIZATION REFRESH ──────────────────────────────────────────
        // Query WMI for GPU Engine utilization. We separate iGPU (Intel integrated)
        // and dGPU (discrete: NVIDIA/AMD) based on the GPU name.
        // If the WMI query fails or no GPU is found, we default to 0%.
        let (igpu_util, dgpu_util) = Self::query_gpu_utilization();
        Self::push_history(&mut self.igpu_history, igpu_util, self.max_history);
        Self::push_history(&mut self.dgpu_history, dgpu_util, self.max_history);
    }

    // Small reusable helper: push a value onto a VecDeque and pop the oldest
    // entry if the deque has grown beyond the desired window length.
    fn push_history(deque: &mut VecDeque<f64>, value: f64, max_len: usize) {
        deque.push_back(value);
        if deque.len() > max_len {
            deque.pop_front();
        }
    }

    // ---------------------------------------------------------------------------
    // GPU UTILIZATION HELPER — Query WMI for GPU Engine utilization percentage
    // ---------------------------------------------------------------------------
    // On Windows, we query WMI for GPU utilization. This is the same data source
    // that Windows Task Manager and Performance Monitor use under the hood.
    //
    // Returns: (igpu_util, dgpu_util) where:
    //   igpu_util = Intel iGPU utilization % (0.0–100.0), or 0.0 if not found
    //   dgpu_util = Discrete GPU utilization % (0.0–100.0), or 0.0 if not found
    //
    // We split iGPU vs dGPU by checking the GPU name:
    //   - iGPU: "Intel" in the description (Intel(R) UHD, Iris Xe, etc.)
    //   - dGPU: "NVIDIA" or "AMD" in the description
    fn query_gpu_utilization() -> (f64, f64) {
        // Initialize COM for WMI (required on Windows for WMI queries).
        // WMI is the Windows Management Instrumentation subsystem, exposed via COM.
        let com_lib = match wmi::COMLibrary::new() {
            Ok(lib) => lib,
            Err(_) => return (0.0, 0.0),
        };

        let wmi_con = match wmi::WMIConnection::new(com_lib) {
            Ok(con) => con,
            Err(_) => return (0.0, 0.0),
        };

        // Try multiple WMI queries for GPU data, in order of preference.
        // Some systems may have different GPU counter configurations.
        let queries = vec![
            // Standard 3D GPU Engine utilization (most common on modern Windows)
            "SELECT Name, UtilizationPercentage FROM Win32_PerfFormattedData_Gpu3d_Gpu3dEngine",
            // Raw data counter variant
            "SELECT Name FROM Win32_PerfRawData_Gpu3d_Gpu3dEngine",
            // Try querying from the root\cimv2 namespace explicitly
            "SELECT * FROM Win32_PerfFormattedData_Gpu3d_Gpu3dEngine WHERE UtilizationPercentage > 0",
        ];

        for query in queries {
            let results: Vec<std::collections::HashMap<String, String>> = match wmi_con.raw_query(query) {
                Ok(res) => res,
                Err(_) => continue, // Try next query
            };

            if results.is_empty() {
                continue; // No results, try next query
            }

            // Successfully got GPU data. Parse it.
            let (mut igpu_max, mut dgpu_max) = (0.0_f64, 0.0_f64);

            for row in results {
                // Extract Name and UtilizationPercentage fields if they exist.
                // For formatted data, UtilizationPercentage should be present.
                // For raw data, we may only have Name (and would need different logic).
                let name = row.get("Name")
                    .map(|s| s.as_str())
                    .unwrap_or("")
                    .to_uppercase();

                // Try to parse UtilizationPercentage. If not available (raw data),
                // default to checking if GPU was found (we'll show non-zero indicator).
                let util: f64 = row.get("UtilizationPercentage")
                    .and_then(|u_str| u_str.parse().ok())
                    .unwrap_or_else(|| {
                        // For raw data queries, if we have a GPU name, assume it's active.
                        if name.contains("INTEL") || name.contains("NVIDIA") || 
                           name.contains("AMD") || name.contains("RADEON") {
                            1.0 // Fallback: show 1% to indicate GPU exists
                        } else {
                            0.0
                        }
                    });

                // Categorize by GPU vendor.
                if name.contains("INTEL") {
                    igpu_max = igpu_max.max(util);
                } else if name.contains("NVIDIA") || name.contains("AMD") || name.contains("RADEON") {
                    dgpu_max = dgpu_max.max(util);
                }
            }

            // If either GPU was found and we got data, return it.
            if igpu_max > 0.0 || dgpu_max > 0.0 {
                return (igpu_max, dgpu_max);
            }
        }

        // All queries returned empty. GPUs may not be exposing WMI counters.
        // Try a fallback: query for video controllers just to detect if GPUs exist.
        // This helps distinguish between "no GPU" and "GPU exists but no utilization data".
        let _gpu_exists: bool = match wmi_con.raw_query::<std::collections::HashMap<String, String>>(
            "SELECT Name FROM Win32_VideoController"
        ) {
            Ok(results) => {
                // If we have video controllers, check if any are Intel/NVIDIA/AMD
                results.iter().any(|gpu| {
                    let name = gpu.get("Name").map(|s| s.as_str()).unwrap_or("").to_uppercase();
                    name.contains("INTEL") || name.contains("NVIDIA") || name.contains("AMD") || name.contains("RADEON")
                })
            }
            Err(_) => false,
        };

        // If GPUs exist but WMI performance counters aren't available, this is likely due to:
        //   - GPU drivers not exposing performance counters via WMI
        //   - Windows Performance Monitor GPU counters not enabled
        //   - Running in a virtual machine or remote session
        // We return 0.0 in this case, but could log a message if error reporting was enabled.
        (0.0, 0.0)
    }
}

// ---------------------------------------------------------------------------
// THE eframe::App TRAIT
// ---------------------------------------------------------------------------
// A "trait" in Rust is like an interface in TypeScript/Go.
// In TypeScript: interface Renderable { render(): void; }
// In Rust:       trait App { fn update(&mut self, ctx: &Context, frame: &mut Frame); }
//
// By implementing `eframe::App` for our `SystemMonitor` struct, we're telling
// eframe: "this struct knows how to draw itself — hand it control every frame."
// eframe will call `update()` on our struct in a loop, driven by the OS's
// window message pump (WndProc / GetMessage loop on Windows).
//
// IMMEDIATE MODE vs RETAINED MODE
// ────────────────────────────────
// Browser (Retained Mode):
//   You create DOM nodes once. The browser keeps them in memory. You update them
//   surgically: document.getElementById('cpu').textContent = '34%'.
//   The browser's render engine diffs the DOM and repaints only what changed.
//
// egui (Immediate Mode):
//   Every frame (~60 fps), you describe the ENTIRE UI from scratch in code.
//   egui does NOT keep widget objects between frames. You call ui.label("CPU: 34%")
//   every frame and egui draws it fresh each time.
//   There is no "update a label" — you just re-run the same code with new data.
//
// Why immediate mode is perfect for real-time monitoring:
//   - No stale state: the UI is always a pure function of your data struct.
//   - No event handlers to wire up: just read your metrics and draw them.
//   - No "forgot to update the UI" bugs: if the data changed, the next frame shows it.
//   - Web analogy: imagine React re-rendering your entire app from scratch every 16ms
//     but doing so efficiently using GPU-accelerated drawing instead of DOM diffing.
impl eframe::App for SystemMonitor {
    // ---------------------------------------------------------------------------
    // THE UPDATE FUNCTION — THE "GAME LOOP"
    // ---------------------------------------------------------------------------
    // `update()` is called by eframe on every frame — typically 60 times per second.
    //
    // WHAT IS A FRAME?
    //   A "frame" is one complete cycle of: process input → update state → draw.
    //   At 60 fps, each frame is ~16.67 milliseconds.
    //   This is the same concept as requestAnimationFrame() in browser JavaScript,
    //   or the Update() function in Unity/Unreal game engines.
    //
    // PARAMETERS:
    //   &mut self  — mutable reference to our app state (self.cpu_history, etc.)
    //   ctx        — the egui context: provides access to drawing APIs, input, etc.
    //   _frame     — eframe window handle (we don't need it for this MVP)
    //
    // Web analogy:
    //   This entire function is like setInterval(() => { fetch('/metrics').then(data => {
    //     updateDOM(data); drawCanvas(data); }); }, 1000) — except:
    //   - There is no DOM. egui redraws everything from scratch each frame.
    //   - The "fetch" is a direct in-process Win32 API call (via sysinfo), not HTTP.
    //   - The "canvas" is a GPU-accelerated retained draw list, not an HTML canvas.
    //   - The loop runs at 60fps, not 1s, but we throttle the metric polling to 1s.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---------------------------------------------------------------------------
        // STEP 1: THROTTLE METRIC UPDATES TO 1 Hz (once per second)
        // ---------------------------------------------------------------------------
        // egui calls update() ~60 times/second. We don't need to poll the OS
        // every frame — that wastes CPU and the data doesn't change that fast anyway.
        //
        // Instant::elapsed() returns how much time has passed since last_update.
        // Duration::from_secs(1) is the threshold — 1 second.
        //
        // Web analogy: think of this as a debounce/throttle guard:
        //   if (Date.now() - lastUpdate > 1000) { fetchMetrics(); lastUpdate = Date.now(); }
        if self.last_update.elapsed() >= Duration::from_secs(1) {
            self.refresh_metrics();
            self.last_update = Instant::now(); // reset the clock
        }

        // ---------------------------------------------------------------------------
        // STEP 2: READ CURRENT VALUES FOR DISPLAY LABELS
        // ---------------------------------------------------------------------------
        // We peek at the most recent value in the deque (the back = newest).
        // back() returns Option<&f64> — None if the deque is empty.
        // unwrap_or(0.0) gives us 0.0 as a safe default before the first reading.
        let current_cpu = *self.cpu_history.back().unwrap_or(&0.0);

        // Convert raw kilobytes to gigabytes for the human-readable label.
        let used_gb = self.system.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_gb = self.system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
        let current_mem_pct = *self.mem_history.back().unwrap_or(&0.0);

        // ---------------------------------------------------------------------------
        // STEP 3: APPLY DARK VISUAL STYLE
        // ---------------------------------------------------------------------------
        // egui's visuals control global colors, rounding, spacing.
        // dark() gives us the built-in dark theme, similar to Task Manager.
        // We clone it, modify individual fields, then apply it.
        let mut visuals = egui::Visuals::dark();
        // panel_fill is the background color for CentralPanel and side panels.
        // Color32::from_rgb(18, 18, 18) is a very dark gray — the Task Manager charcoal.
        visuals.panel_fill = Color32::from_rgb(18, 18, 18);
        ctx.set_visuals(visuals);

        // ---------------------------------------------------------------------------
        // STEP 4: DRAW THE UI
        // ---------------------------------------------------------------------------
        // CentralPanel fills the entire window. Think of it like:
        //   <div style="width:100%; height:100%; display:flex; flex-direction:column;">
        // The closure receives a `ui` handle — the drawing context for this panel.
        egui::CentralPanel::default().show(ctx, |ui| {
            // ScrollArea lets the user scroll vertically when the window is too
            // short to show all graphs at once — like overflow-y: auto in CSS.
            egui::ScrollArea::vertical().show(ui, |ui| {
            // Add some breathing room around all content, like CSS padding.
            ui.add_space(8.0);

            // ── TIME RANGE SELECTOR ──────────────────────────────────────────
            // Display a row of selectable buttons that control how many seconds
            // of history all graphs display. Only one can be active at a time
            // (radio-button behaviour).
            //
            // egui's selectable_value() works like a radio button group:
            //   ui.selectable_value(&mut state_var, candidate_value, label)
            //   • If state_var == candidate_value → renders with a filled/highlighted
            //     background (the "selected" look).
            //   • If state_var != candidate_value → renders with a subtle outline
            //     (the "unselected" look).
            //   • On click → sets state_var = candidate_value automatically.
            // Because all buttons share the same &mut variable (selected_duration),
            // selecting one deselects the others — exactly like HTML radio inputs.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Time Range:")
                        .size(14.0)
                        .color(Color32::from_rgb(180, 180, 180)),
                );
                for &(label, duration) in &[
                    ("30s", 30u64), ("1m", 60), ("5m", 300),
                    ("10m", 600), ("30m", 1800), ("1h", 3600),
                ] {
                    ui.selectable_value(&mut self.selected_duration, duration, label);
                }
            });

            // Sync the display window size with the (possibly just-changed)
            // selected duration. This runs every frame so graphs immediately
            // reflect any change from the buttons above.
            self.history_length = self.selected_duration as usize;

            // Human-readable X axis label reflecting the selected time range.
            let x_label = match self.selected_duration {
                30   => "Last 30 seconds",
                60   => "Last 1 minute",
                300  => "Last 5 minutes",
                600  => "Last 10 minutes",
                1800 => "Last 30 minutes",
                3600 => "Last 1 hour",
                _    => "Time",
            };

            ui.add_space(8.0);

            // ── CPU CARD ─────────────────────────────────────────────────────
            // egui::Frame is a container widget — similar to a <div> with CSS
            // border and padding in web development. It draws a bordered, padded
            // box around its child widgets, visually grouping them into a "card".
            //
            // Frame::group(ui.style()) uses the theme's "group" style which
            // provides a subtle rounded border and internal padding. Compare to
            // Frame::none() (invisible) or Frame::canvas() (opaque fill) — group()
            // strikes the best balance for card-style layouts.
            //
            // NOTE: This card structure is the foundation for drag-and-drop and
            // flexbox-like reordering of metric panels planned for a future increment.
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new(format!("CPU Usage  —  {:.1}%", current_cpu))
                        .size(16.0)
                        .color(Color32::from_rgb(100, 180, 255)),
                );
                ui.add_space(6.0);

                // To display only the selected time window from the full 3600-point
                // buffer, we take the last `history_length` elements from the VecDeque.
                // VecDeque::iter() yields items front-to-back (oldest → newest).
                // By computing skip = total_items − window_size and calling .skip(skip),
                // we start iterating from the (total − window)th element — i.e. we get
                // only the most recent `window` data points. After skip, enumerate()
                // produces 0-based X coordinates: x=0 is the oldest visible point,
                // x=window−1 is the newest.
                let window = self.history_length.min(self.cpu_history.len());
                let skip = self.cpu_history.len() - window;
                let cpu_points: PlotPoints = self
                    .cpu_history
                    .iter()
                    .skip(skip)
                    .enumerate()
                    .map(|(i, &val)| [i as f64, val])
                    .collect();

                let cpu_line = Line::new(cpu_points)
                    .color(Color32::from_rgb(70, 140, 255))
                    .width(2.0);

                // egui_plot auto-scales axes by default to fit the visible data.
                // This means if all values hover around 30%, the Y axis might zoom
                // in to 25–35%, hiding the full 0–100 context. include_y(0.0) and
                // include_y(100.0) force the Y axis to always span the full range.
                // Performance metrics (CPU, memory) are physical quantities that
                // can never go below 0% or above 100%, so locking the range is correct.
                // allow_scroll(false) prevents mouse-wheel scrolling which could
                // shift the view into negative territory, defeating the include_y pins.
                Plot::new("cpu_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(100.0)
                    .y_axis_label("% Usage")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(cpu_line);
                    });
            });
            ui.add_space(8.0);

            // ── MEMORY CARD ────────────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new(format!(
                        "Memory Usage  —  {:.1}%  ({:.1} GB / {:.1} GB)",
                        current_mem_pct, used_gb, total_gb
                    ))
                        .size(16.0)
                        .color(Color32::from_rgb(100, 220, 130)),
                );
                ui.add_space(6.0);

                // Slice the last `history_length` points from the buffer for display.
                let window = self.history_length.min(self.mem_history.len());
                let skip = self.mem_history.len() - window;
                let mem_points: PlotPoints = self
                    .mem_history
                    .iter()
                    .skip(skip)
                    .enumerate()
                    .map(|(i, &val)| [i as f64, val])
                    .collect();

                let mem_line = Line::new(mem_points)
                    .color(Color32::from_rgb(80, 210, 110))
                    .width(2.0);

                // Y axis locked 0–100: memory percentage can never be negative.
                // allow_scroll(false) prevents scrolling into negative territory.
                Plot::new("mem_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(100.0)
                    .y_axis_label("% Used")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(mem_line);
                    });
            });
            ui.add_space(8.0);

            // ── DISK C: CARD ───────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new("Disk C:  —  I/O Rate")
                        .size(16.0)
                        .color(Color32::from_rgb(255, 180, 80)),
                );
                ui.add_space(4.0);

                let c_read_now  = *self.disk_c_read_history.back().unwrap_or(&0.0);
                let c_write_now = *self.disk_c_write_history.back().unwrap_or(&0.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Read:  {:.2} MB/s", c_read_now))
                            .size(18.0)
                            .color(Color32::from_rgb(255, 180, 80)),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(format!("Write: {:.2} MB/s", c_write_now))
                            .size(18.0)
                            .color(Color32::from_rgb(220, 80, 80)),
                    );
                });
                ui.add_space(6.0);

                // Slice the last `history_length` points for display.
                let window = self.history_length.min(self.disk_c_read_history.len());
                let skip_r = self.disk_c_read_history.len() - window;
                let c_read_pts: PlotPoints = self.disk_c_read_history
                    .iter().skip(skip_r).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();
                let skip_w = self.disk_c_write_history.len().saturating_sub(window);
                let c_write_pts: PlotPoints = self.disk_c_write_history
                    .iter().skip(skip_w).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();

                let c_read_line  = Line::new(c_read_pts)
                    .color(Color32::from_rgb(255, 180, 80))
                    .width(2.0)
                    .name("Read");
                let c_write_line = Line::new(c_write_pts)
                    .color(Color32::from_rgb(220, 80, 80))
                    .width(2.0)
                    .name("Write");

                Plot::new("disk_c_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(1.0)
                    .y_axis_label("MB/s")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(c_read_line);
                        plot_ui.line(c_write_line);
                    });
            });
            ui.add_space(8.0);

            // ── DISK D: CARD ───────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new("Disk D:  —  I/O Rate")
                        .size(16.0)
                        .color(Color32::from_rgb(255, 220, 80)),
                );
                ui.add_space(4.0);

                let d_read_now  = *self.disk_d_read_history.back().unwrap_or(&0.0);
                let d_write_now = *self.disk_d_write_history.back().unwrap_or(&0.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Read:  {:.2} MB/s", d_read_now))
                            .size(18.0)
                            .color(Color32::from_rgb(255, 220, 80)),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(format!("Write: {:.2} MB/s", d_write_now))
                            .size(18.0)
                            .color(Color32::from_rgb(180, 100, 220)),
                    );
                });
                ui.add_space(6.0);

                let window = self.history_length.min(self.disk_d_read_history.len());
                let skip_r = self.disk_d_read_history.len() - window;
                let d_read_pts: PlotPoints = self.disk_d_read_history
                    .iter().skip(skip_r).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();
                let skip_w = self.disk_d_write_history.len().saturating_sub(window);
                let d_write_pts: PlotPoints = self.disk_d_write_history
                    .iter().skip(skip_w).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();

                let d_read_line  = Line::new(d_read_pts)
                    .color(Color32::from_rgb(255, 220, 80))
                    .width(2.0)
                    .name("Read");
                let d_write_line = Line::new(d_write_pts)
                    .color(Color32::from_rgb(180, 100, 220))
                    .width(2.0)
                    .name("Write");

                Plot::new("disk_d_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(1.0)
                    .y_axis_label("MB/s")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(d_read_line);
                        plot_ui.line(d_write_line);
                    });
            });
            ui.add_space(8.0);

            // ── NETWORK CARD ──────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new("Network  —  Total (all adapters)")
                        .size(16.0)
                        .color(Color32::from_rgb(80, 220, 240)),
                );
                ui.add_space(4.0);

                let net_recv_now = *self.net_recv_history.back().unwrap_or(&0.0);
                let net_sent_now = *self.net_sent_history.back().unwrap_or(&0.0);

                let fmt_net = |kbs: f64| -> String {
                    if kbs >= 1024.0 {
                        format!("{:.2} MB/s", kbs / 1024.0)
                    } else {
                        format!("{:.1} KB/s", kbs)
                    }
                };

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{25bc} Download: {}", fmt_net(net_recv_now)))
                            .size(18.0)
                            .color(Color32::from_rgb(80, 220, 240)),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new(format!("\u{25b2} Upload:   {}", fmt_net(net_sent_now)))
                            .size(18.0)
                            .color(Color32::from_rgb(240, 130, 200)),
                    );
                });
                ui.add_space(6.0);

                let window = self.history_length.min(self.net_recv_history.len());
                let skip_r = self.net_recv_history.len() - window;
                let net_recv_pts: PlotPoints = self.net_recv_history
                    .iter().skip(skip_r).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();
                let skip_s = self.net_sent_history.len().saturating_sub(window);
                let net_sent_pts: PlotPoints = self.net_sent_history
                    .iter().skip(skip_s).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();

                let net_recv_line = Line::new(net_recv_pts)
                    .color(Color32::from_rgb(80, 220, 240))
                    .width(2.0)
                    .name("Download");
                let net_sent_line = Line::new(net_sent_pts)
                    .color(Color32::from_rgb(240, 130, 200))
                    .width(2.0)
                    .name("Upload");

                Plot::new("net_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(10.0)
                    .y_axis_label("KB/s")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(net_recv_line);
                        plot_ui.line(net_sent_line);
                    });
            });
            ui.add_space(8.0);

            // ── iGPU CARD ─────────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let igpu_util = *self.igpu_history.back().unwrap_or(&0.0);
                ui.heading(
                    egui::RichText::new(format!("GPU  —  Intel iGPU  —  {:.1}%", igpu_util))
                        .size(16.0)
                        .color(Color32::from_rgb(100, 180, 255)),
                );
                ui.add_space(6.0);

                let window = self.history_length.min(self.igpu_history.len());
                let skip = self.igpu_history.len() - window;
                let igpu_pts: PlotPoints = self.igpu_history
                    .iter().skip(skip).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();

                let igpu_line = Line::new(igpu_pts)
                    .color(Color32::from_rgb(100, 180, 255))
                    .width(2.0);

                Plot::new("igpu_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(100.0)
                    .y_axis_label("% Used")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(igpu_line);
                    });
            });
            ui.add_space(8.0);

            // ── dGPU CARD ─────────────────────────────────────────────────
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let dgpu_util = *self.dgpu_history.back().unwrap_or(&0.0);
                ui.heading(
                    egui::RichText::new(format!("GPU  —  Discrete (NVIDIA/AMD)  —  {:.1}%", dgpu_util))
                        .size(16.0)
                        .color(Color32::from_rgb(120, 200, 140)),
                );
                ui.add_space(6.0);

                let window = self.history_length.min(self.dgpu_history.len());
                let skip = self.dgpu_history.len() - window;
                let dgpu_pts: PlotPoints = self.dgpu_history
                    .iter().skip(skip).enumerate()
                    .map(|(i, &v)| [i as f64, v])
                    .collect();

                let dgpu_line = Line::new(dgpu_pts)
                    .color(Color32::from_rgb(120, 200, 140))
                    .width(2.0);

                Plot::new("dgpu_plot")
                    .height(140.0)
                    .include_y(0.0)
                    .include_y(100.0)
                    .y_axis_label("% Used")
                    .x_axis_label(x_label)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                    .show(ui, |plot_ui| {
                        plot_ui.line(dgpu_line);
                    });
            });
            ui.add_space(8.0);
            }); // end ScrollArea
        });

        // ---------------------------------------------------------------------------
        // STEP 5: REQUEST THE NEXT REPAINT
        // ---------------------------------------------------------------------------
        // By default, egui only redraws when there is user input (mouse, keyboard).
        // For a monitoring app, we need the window to update even when idle.
        //
        // ctx.request_repaint() tells the eframe event loop: "schedule another frame
        // as soon as possible." This keeps update() being called ~60 times per second
        // regardless of user activity.
        //
        // Web analogy: this is equivalent to calling requestAnimationFrame() at the
        // END of your animation callback to schedule the next frame — without it,
        // animation would stop after the first paint.
        //
        // Note: this doesn't poll the OS every frame — our throttle guard above
        // (last_update check) ensures we only call sysinfo once per second.
        // The extra frames just redraw the existing data at smooth 60fps.
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// MAIN FUNCTION — PROGRAM ENTRY POINT
// ---------------------------------------------------------------------------
// `fn main()` is always where a Rust binary starts. No `async`, no framework
// magic — the OS calls main() directly. Web analogy: like index.html loading
// your script and calling init() — except there is no HTML at all.
fn main() -> eframe::Result<()> {
    // NativeOptions configures the underlying OS window.
    // This is like calling: window.resizeTo(900, 600) + setting the window title
    // in a browser — except we're talking to the Win32 CreateWindowEx() API.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("System Monitor")      // Title bar text
            .with_inner_size([900.0, 1100.0])   // Initial window size in logical pixels
            .with_min_inner_size([400.0, 300.0]) // Minimum resize boundary
            .with_resizable(true),             // Allow the user to drag window edges
        ..Default::default() // All other options use safe defaults
    };

    // eframe::run_native() hands control to eframe's event loop.
    // This is a BLOCKING call — it only returns when the user closes the window.
    // Inside, eframe runs the Windows message pump:
    //   while GetMessage(&msg, ...) { TranslateMessage(&msg); DispatchMessage(&msg); }
    // On each paint message, it calls our update() function.
    //
    // The third argument is a Box<dyn FnOnce> factory — eframe calls it once to
    // create our app struct. Using Box<dyn ...> here is eframe's way of accepting
    // any type that implements App without knowing the concrete type at compile time.
    // Web analogy: like passing a class constructor to a framework:
    //   ReactDOM.render(<App />, document.getElementById('root'))
    // In eframe 0.27, the factory closure returns Box<dyn App> directly (not Result).
    // Later versions of eframe changed this to return Result — but 0.27 uses the simpler form.
    eframe::run_native(
        "System Monitor",
        options,
        Box::new(|_cc| Box::new(SystemMonitor::new()) as Box<dyn eframe::App>),
    )
}

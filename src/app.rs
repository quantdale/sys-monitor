// ---------------------------------------------------------------------------
// IMPORTS
// ---------------------------------------------------------------------------
use eframe::egui;
use egui::Color32;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use sysinfo::System;
use sysinfo::Disks;
use sysinfo::Networks;
use windows::Win32::System::Performance::PdhCollectQueryData;

// ---------------------------------------------------------------------------
// APP STATE STRUCT
// ---------------------------------------------------------------------------
// In egui's immediate-mode model, YOU own all state. There is no hidden DOM
// or component tree storing values. You define a struct that holds everything
// your app needs between frames, and you pass it to egui each frame.
//
// Web analogy: This is like your React component's `useState` or a Svelte store —
// it's the single source of truth for everything the UI will display.
pub(crate) struct SystemMonitor {
    // sysinfo::System is the main entry point to the sysinfo crate.
    // Under the hood on Windows it:
    //   - For CPU: opens PDH (Performance Data Helper) counters, specifically
    //     the "\Processor(_Total)\% Processor Time" counter. This is the same
    //     counter that Task Manager reads. PDH is a Win32 subsystem.
    //   - For Memory: calls GlobalMemoryStatusEx(), a Win32 API function that
    //     returns a MEMORYSTATUSEX struct with dwTotalPhys and dwAvailPhys fields.
    //     Used Memory = Total - Available.
    // sysinfo wraps all this Win32 complexity into clean, safe Rust.
    pub(crate) system: System,

    // Sliding window of the last N CPU usage samples (0.0 – 100.0).
    // We store f64 (64-bit float) because egui_plot expects f64 coordinates.
    pub(crate) cpu_history: VecDeque<f64>,

    // Same sliding window for memory usage, stored as a percentage (0.0 – 100.0)
    // so both graphs share the same Y-axis scale.
    pub(crate) mem_history: VecDeque<f64>,

    // std::time::Instant is a monotonic clock timestamp — it only goes forward,
    // immune to system clock changes (unlike wall-clock time).
    // Web analogy: performance.now() returns milliseconds since page load.
    //              Instant::now() returns an opaque point in time.
    //              elapsed() gives you the Duration since that point.
    // We use this to throttle our system polling to once per second.
    pub(crate) last_update: Instant,

    // Number of data points to display in the current time window.
    // Dynamically set by the time range selector (30, 60, 300, 600, 1800, 3600).
    pub(crate) history_length: usize,

    // The maximum capacity of the history buffer — always 3600 (one data point
    // per second for 1 full hour). The buffer must be this large regardless of
    // the user's currently selected time range, because the user might switch
    // from "30s" to "1h" at any time and expects to see all data that was
    // silently collected in the background. If the buffer were capped at the
    // display window size, switching to a longer range would show gaps.
    pub(crate) max_history: usize,

    // The currently selected duration in seconds, controlled by the header
    // buttons (30, 60, 300, 600, 1800, 3600). Default: 60 (1 minute).
    pub(crate) selected_duration: u64,

    // ── DISK I/O ────────────────────────────────────────────────────────────
    // sysinfo::Disks is a dedicated handle for querying disk statistics.
    // It is separate from System because disk I/O polling is relatively expensive —
    // separating it lets you refresh disks independently of CPU/RAM.
    pub(crate) disks: Disks,

    // Per-physical-disk active-time history (%), keyed by drive-letter group
    // such as "C:" or "C: D:". Values are always in the 0.0-100.0 range.
    //
    // Why % active time instead of MB/s throughput:
    //   - Sequential I/O can produce high MB/s with low busy time (fast NVMe)
    //   - Random 4K I/O can pin the disk at 100% busy with low MB/s
    // % Disk Time measures the fraction of elapsed time with pending requests,
    // which is the utilization metric Task Manager's disk graph displays.
    pub(crate) disk_active_histories: HashMap<String, VecDeque<f64>>,
    pub(crate) disk_display_order: Vec<String>,

    // ── NETWORK I/O ──────────────────────────────────────────────────────────
    // Networks is a HashMap<String, NetworkData> under the hood — each key is
    // the OS-assigned interface name (e.g. "Ethernet", "Wi-Fi", "vEthernet").
    // We keep one handle and call refresh() on it every second.
    pub(crate) networks: Networks,

    // Combined download (received) and upload (transmitted) rate across all
    // non-loopback interfaces, stored in KB/s.
    // Aggregating all adapters matches Task Manager's "Total" network view.
    // We use KB/s rather than MB/s for network because most traffic is in the
    // tens-of-KB/s range; MB/s would make the graph flat most of the time.
    pub(crate) net_recv_history: VecDeque<f64>, // download KB/s history
    pub(crate) net_sent_history: VecDeque<f64>, // upload   KB/s history

    // ── GPU UTILIZATION ─────────────────────────────────────────────────────────
    // We track iGPU (Intel integrated) and dGPU (discrete: NVIDIA/AMD) separately.
    // On Windows, we query WMI for GPU Engine utilization percentage via
    // Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine — the same class
    // that Windows Task Manager's GPU tab uses. iGPU typically reports as
    // "Intel(R) UHD Graphics" or similar; dGPU as "NVIDIA GeForce RTX" etc.
    //
    // If a GPU is not present, its history remains empty (displays 0%).
    // Utilization is stored as a percentage (0.0 – 100.0).
    pub(crate) igpu_history: VecDeque<f64>, // Intel iGPU utilization % history
    pub(crate) dgpu_history: VecDeque<f64>, // Discrete GPU (NVIDIA/AMD) utilization % history

    // ── GPU DEBUG FLAG ───────────────────────────────────────────────────────────
    // When true, every raw WMI row returned by the GPU query is printed to stderr.
    // Useful for diagnosing why GPU readings might be zero on a specific machine.
    // Toggle off once GPU readings are confirmed working.
    // To see output: run with `cargo run` in a terminal (not double-clicking .exe).
    pub(crate) gpu_debug: bool,

    // ── GPU ERROR FLAG ───────────────────────────────────────────────────────────
    // When true, the first GPU error has already been printed to stderr.
    // Prevents spamming the same "[GPU] WMI query failed" message every second.
    pub(crate) gpu_error_logged: bool,

    // ── WMI CONNECTION ───────────────────────────────────────────────────────────
    // Persistent WMI connection used for all GPU counter queries.
    //
    // WHY PERSISTENT — RATE-BASED COUNTERS NEED TWO SAMPLES:
    //   Win32_PerfFormattedData_* classes are rate-based performance counters.
    //   Windows computes utilization as: (value₂ − value₁) / time_delta.
    //   This delta is maintained PER-CONNECTION on the WMI provider side.
    //   If we create a new WMIConnection on every poll, Windows discards the
    //   previous baseline and starts over from zero — the delta is never computed
    //   and every query returns 0, regardless of actual GPU load:
    //
    //     WRONG (new connection every poll):
    //       Poll 1: new WMIConnection → sample 1 → baseline set → returns 0
    //       Poll 2: new WMIConnection → sample 1 again → baseline discarded → 0
    //       Poll 3: same → returns 0 forever
    //
    //     CORRECT (persistent connection, this implementation):
    //       Poll 1: same WMIConnection → sample 1 → baseline set → returns 0
    //       Poll 2: same WMIConnection → sample 2 → delta computed → real %
    //       Poll 3: same WMIConnection → sample 3 → rolling delta → real %
    //
    //   This is the same reason Task Manager's GPU graph shows 0 for the first
    //   second and then starts updating — it needs one baseline sample first.
    //
    //   CPU from sysinfo is unaffected: sysinfo manages its own persistent internal
    //   state and handles the delta internally, so there is no equivalent issue.
    //   Disk I/O and network rates from sysinfo are likewise handled internally.
    //
    // WHY Option<WMIConnection>:
    //   Option<T> is Rust's equivalent of a nullable type (like `T | null` in
    //   TypeScript). We initialize it once in new(); if that fails (rare, but
    //   possible on some Windows configurations), we store None and show 0%
    //   rather than panicking. The one-shot gpu_error_logged flag ensures the
    //   error is printed once rather than every poll cycle.
    pub(crate) wmi_con: Option<wmi::WMIConnection>,

    // ── PDH GPU HANDLES ──────────────────────────────────────────────────────────
    // PDH (Performance Data Helper) is the Win32 performance counter API that
    // Windows Task Manager uses directly for GPU metrics.
    //
    // PDH LIFECYCLE — open once, never recreate mid-session:
    //   1. PdhOpenQueryW          — creates a container for a group of counters
    //   2. PdhAddEnglishCounterW  — registers counter paths (wildcard-enabled)
    //   3. PdhCollectQueryData    — snapshots all counters atomically (every poll)
    //   4. PdhGetFormattedCounterArrayW — reads per-instance formatted values
    //
    //   The baseline for computing utilization rates is maintained INTERNALLY by
    //   the PDH query handle. Recreating the handle resets the baseline to zero,
    //   causing the same always-zero problem that existed with WMI connections.
    //
    // PDH_HQUERY — opaque handle to a group of counters monitored together.
    //   All counters in one query are snapshotted atomically by PdhCollectQueryData.
    //   Think of it as a "query session" that tracks baseline state across polls.
    //
    // PDH_HCOUNTER — handle to one specific counter within a query.
    //   Each counter uses a wildcard instance filter (e.g. `*engtype_3D*`) that PDH
    //   expands to all matching per-process/per-adapter instances at collection time.
    //
    // Both are Option<> so the app degrades gracefully (shows 0%) if PDH init fails.
    pub(crate) pdh_query: Option<isize>,
    pub(crate) pdh_gpu_3d_counter: Option<isize>,    // \GPU Engine(*engtype_3D*)\Utilization Percentage
    #[allow(dead_code)] // stored for future VideoDecode UI display; not yet read
    pub(crate) pdh_gpu_video_counter: Option<isize>, // \GPU Engine(*engtype_VideoDecode*)\Utilization Percentage
    pub(crate) pdh_disk_active_counter: Option<isize>, // \PhysicalDisk(*)\% Idle Time  (inverted → active%)
}

impl SystemMonitor {
    // `new()` is Rust's convention for a constructor. There's no `new` keyword —
    // it's just a static method that returns Self (the type being implemented).
    // Web analogy: This is like a class constructor in TypeScript:
    //   constructor() { this.system = new System(); this.cpuHistory = []; ... }
    pub fn new() -> Self {
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

        // Initialize the disk handle for mount-point metadata (drive-letter mapping).
        // Disk utilization itself comes from PDH % Disk Time, not sysinfo byte deltas.
        let mut disks = Disks::new_with_refreshed_list();
        disks.refresh(false);

        // Initialise the Networks handle.
        // new_with_refreshed_list() calls GetIfTable2() immediately to discover all
        // interfaces and seed the internal counters, so the first refresh() call
        // will already produce a valid delta (not bytes-since-boot).
        let mut networks = Networks::new_with_refreshed_list();
        // A second refresh seeds the per-interface delta counters so the first
        // reading we display is sensible rather than a large spike from boot stats.
        networks.refresh(false);

        // Initialize PDH GPU counter handles once at startup.
        // PDH (Performance Data Helper) — the same API Windows Task Manager uses.
        // We open the query here so the handle persists for the app lifetime,
        // enabling PDH to maintain its internal baseline for rate computation.
        // new_pdh_gpu_query() also makes a first PdhCollectQueryData call to
        // establish the baseline; real readings start on the second poll (~1s later).
        let (
            pdh_query,
            pdh_gpu_3d_counter,
            pdh_gpu_video_counter,
            pdh_disk_active_counter,
        ) =
            match crate::platform::pdh::new_pdh_gpu_query() {
                Some((q, c3d, cvid, cdisk)) => (Some(q), Some(c3d), cvid, cdisk),
                None => (None, None, None, None),
            };

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
            disk_active_histories: HashMap::new(),
            disk_display_order: Vec::new(),
            networks,
            net_recv_history: VecDeque::with_capacity(3600),
            net_sent_history: VecDeque::with_capacity(3600),
            igpu_history: VecDeque::with_capacity(3600),
            dgpu_history: VecDeque::with_capacity(3600),
            gpu_debug: false, // set to true to print raw WMI rows to stderr
            gpu_error_logged: false, // set to true after the first GPU error is logged

            // Initialize the persistent WMI connection once at startup.
            //
            // assume_initialized() is required because eframe's windowing backend
            // (winit) has already called CoInitializeEx(COINIT_APARTMENTTHREADED)
            // on this thread before the app_creator closure (which calls new()) runs.
            // COMLibrary::new() would call CoInitializeEx again with MTA, which
            // Windows rejects with RPC_E_CHANGED_MODE (0x80010106).
            //
            // NOTE: The very first GPU poll after launch will still return 0% —
            // this is CORRECT and EXPECTED. WMI PerfFormattedData requires one
            // baseline sample before it can compute a utilization delta. Real
            // readings begin on the second poll (~1 second after launch).
            wmi_con: {
                let com = unsafe { wmi::COMLibrary::assume_initialized() };
                match wmi::WMIConnection::new(com) {
                    Ok(con) => {
                        eprintln!("[WMI] Connection initialized successfully.");
                        Some(con)
                    }
                    Err(e) => {
                        eprintln!("[WMI] Failed to initialize connection: {:?}. GPU metrics unavailable.", e);
                        None
                    }
                }
            },
            pdh_query,
            pdh_gpu_3d_counter,
            pdh_gpu_video_counter,
            pdh_disk_active_counter,
        }
    }

    // Polls the OS for fresh CPU and memory data and pushes it into our history.
    // This is only called once per second (throttled by last_update check in update()).
    pub(crate) fn refresh_metrics(&mut self) {
        crate::metrics::cpu::refresh_cpu(self);
        crate::metrics::memory::refresh_memory(self);
        crate::metrics::network::refresh_network(self);

        // ── DISK + GPU UTILIZATION REFRESH (SHARED PDH SNAPSHOT) ───────────
        // Collect once per poll on the shared PDH query so GPU and disk counters
        // are sampled from the same timestamped snapshot. We do not open a second
        // query for disk because PDH baselines are query-handle scoped.
        let pdh_collected_ok = match self.pdh_query {
            Some(query) => unsafe { PdhCollectQueryData(query) == 0 },
            None => false,
        };

        if pdh_collected_ok {
            crate::metrics::disk::refresh_disk(self);
        }

        // ── GPU UTILIZATION REFRESH ──────────────────────────────────────────
        // Query PDH for GPU Engine utilization. Uses PDH (Performance Data Helper)
        // directly — the same API Windows Task Manager uses for its GPU graphs.
        // We separate iGPU (Intel integrated) and dGPU (discrete: NVIDIA/AMD)
        // based on the LUID-to-vendor map from Win32_VideoController.
        // If PDH init failed or no GPU is found, we default to 0%.
        let (igpu_util, dgpu_util) = crate::metrics::gpu::query_gpu_utilization_pdh(self);
        Self::push_history(&mut self.igpu_history, igpu_util, self.max_history);
        Self::push_history(&mut self.dgpu_history, dgpu_util, self.max_history);
    }

    // Small reusable helper: push a value onto a VecDeque and pop the oldest
    // entry if the deque has grown beyond the desired window length.
    pub fn push_history(deque: &mut VecDeque<f64>, value: f64, max_len: usize) {
        deque.push_back(value);
        if deque.len() > max_len {
            deque.pop_front();
        }
    }

    // ---------------------------------------------------------------------------
    // GPU UTILIZATION — Main entry point: returns (igpu_util%, dgpu_util%)
    // ---------------------------------------------------------------------------
    // Orchestrates build_gpu_vendor_map() and query_gpu_perf_counters(), then
    // classifies each engine row as iGPU (Intel) or dGPU (NVIDIA/AMD) and returns
    // the maximum utilization seen in each category.
    //
    // Returns: (igpu_util, dgpu_util) in the 0.0–100.0 range.
    //   Both values are 0.0 if the GPU is idle, absent, or counters unavailable.
    #[allow(dead_code)] // preserved WMI fallback; PDH is now the primary GPU source
    fn query_gpu_utilization(&mut self) -> (f64, f64) {
        // Temporarily move the WMIConnection out of self so we can pass it by
        // reference to the helpers while also calling &mut self methods.
        //
        // WHY take() / put-back pattern:
        //   Rust's borrow checker prevents holding a shared borrow into self.wmi_con
        //   (&WMIConnection) while simultaneously calling &mut self methods.
        //   Option::take() moves the value out of self.wmi_con (setting it to None),
        //   so the field is no longer borrowed and &mut self calls are allowed.
        //   We restore the connection into self.wmi_con before returning so the
        //   NEXT poll has the persistent baseline needed to compute a delta.
        let wmi_con = match self.wmi_con.take() {
            Some(con) => con,
            None => {
                // WMI connection was never established (init failed in new()).
                // Error already printed once at startup — return silent zeros.
                return (0.0, 0.0);
            }
        };

        // Step 1: LUID → vendor name map (Win32_VideoController, name lookup only).
        // Win32_VideoController is a STATIC info class — it has GPU name, driver
        // version, VRAM size. It has NO utilization data and must never be used
        // as a utilization source. It is only used here to identify vendor names.
        let vendor_map = crate::platform::wmi::build_gpu_vendor_map(&wmi_con, self.gpu_debug);

        // Step 2: live utilization (summed across all PIDs) from GPUPerformanceCounters.
        // Returns two vecs: 3D engine totals and VideoDecode engine totals per LUID.
        //
        // FIRST POLL NOTE: the first call after app launch will return 0% for all LUIDs.
        // This is CORRECT and EXPECTED. WMI PerfFormattedData requires one baseline
        // sample before it can compute a utilization delta. Real readings begin on
        // the second poll (~1 second after launch).
        let (utilization_3d, _utilization_video) = crate::platform::wmi::query_gpu_perf_counters(
            &wmi_con,
            self.gpu_debug,
            &mut self.gpu_error_logged,
        );

        // Restore the connection — must happen before any early returns below.
        // Without this, self.wmi_con stays None and every subsequent poll returns
        // (0.0, 0.0), defeating the persistent-connection fix entirely.
        self.wmi_con = Some(wmi_con);

        // Step 3: classify each LUID as iGPU or dGPU using vendor keywords + fallback.
        // Take the MAX across all same-class GPUs: if both Intel LUIDs are iGPU,
        // we show the busier one (the Xe adapter doing 3D work, not the GDI adapter).
        let mut igpu_max = 0.0f64;
        let mut dgpu_max = 0.0f64;

        for (luid, util) in &utilization_3d {
            match crate::platform::gpu::classify_luid(luid, &vendor_map) {
                crate::platform::gpu::GpuClass::IGpu => igpu_max = igpu_max.max(*util),
                crate::platform::gpu::GpuClass::DGpu => dgpu_max = dgpu_max.max(*util),
                crate::platform::gpu::GpuClass::Unknown => {
                    if !self.gpu_error_logged {
                        eprintln!("[GPU] Unclassified LUID: {} (util={:.1}%)", luid, util);
                    }
                }
            }
        }

        if self.gpu_debug {
            eprintln!(
                "[GPU DEBUG] Final: igpu_max={:.1}%, dgpu_max={:.1}%, vendor_map={:?}",
                igpu_max, dgpu_max, vendor_map
            );
        }

        (igpu_max, dgpu_max)
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
        // STEP 2: APPLY DARK VISUAL STYLE
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
        // STEP 3: DRAW THE UI
        // ---------------------------------------------------------------------------
        // CentralPanel fills the entire window. Think of it like:
        //   <div style="width:100%; height:100%; display:flex; flex-direction:column;">
        // The closure receives a `ui` handle — the drawing context for this panel.
        egui::CentralPanel::default().show(ctx, |ui| {
            crate::render::layout::render_layout(ui, self);
        });

        // ---------------------------------------------------------------------------
        // STEP 4: REQUEST THE NEXT REPAINT
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

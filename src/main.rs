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
use eframe::egui;                     // The immediate-mode GUI widgets
use egui::Color32;                    // RGB color type
use egui_plot::{Line, Plot, PlotPoints}; // The graphing widgets
use std::collections::VecDeque;       // Double-ended queue — explained below
use std::time::{Duration, Instant};   // Monotonic clock, like performance.now() in JS
use sysinfo::System;                  // Wraps Windows system APIs

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

    // Maximum number of data points to keep. 60 = last 60 seconds at 1 Hz.
    history_length: usize,
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
            sysinfo::RefreshKind::new()
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

        SystemMonitor {
            system,
            cpu_history: VecDeque::with_capacity(60), // pre-allocate for 60 items
            mem_history: VecDeque::with_capacity(60),
            // Instant::now() captures "right now". We subtract a full second so
            // the very first frame immediately triggers a data refresh instead of
            // waiting one second for the first reading.
            last_update: Instant::now() - Duration::from_secs(1),
            history_length: 60,
        }
    }

    // Polls the OS for fresh CPU and memory data and pushes it into our history.
    // This is only called once per second (throttled by last_update check in update()).
    fn refresh_metrics(&mut self) {
        // Refresh CPU counters. sysinfo re-queries the PDH counter and computes
        // (new_idle_time - old_idle_time) / elapsed to get a CPU usage percentage.
        self.system.refresh_cpu_usage();

        // Refresh memory counters. sysinfo calls GlobalMemoryStatusEx() again.
        self.system.refresh_memory();

        // global_cpu_info() returns a Cpu struct representing the aggregate "all cores" CPU.
        // Calling .cpu_usage() on it gives the average usage % across ALL logical cores.
        // Example: if you have 8 cores and average utilization is 25%, this returns 25.0.
        let cpu_pct = self.system.global_cpu_info().cpu_usage() as f64;

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

        // If we've exceeded the window size, pop the FRONT (oldest value).
        // This is the core ring-buffer pattern: push_back + pop_front = sliding window.
        if self.cpu_history.len() > self.history_length {
            self.cpu_history.pop_front();
        }
        if self.mem_history.len() > self.history_length {
            self.mem_history.pop_front();
        }
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
            // Add some breathing room around all content, like CSS padding.
            ui.add_space(8.0);

            // ── CPU SECTION ──────────────────────────────────────────────────────
            // RichText lets us style the text — font size, color, bold, etc.
            ui.heading(
                egui::RichText::new("CPU Usage")
                    .size(16.0)
                    .color(Color32::from_rgb(100, 180, 255)), // light blue
            );
            ui.add_space(4.0);

            // Current value as a text label.
            // format!() is Rust's equivalent of template literals: `CPU: ${value.toFixed(1)}%`
            ui.label(
                egui::RichText::new(format!("{:.1}%", current_cpu))
                    .size(26.0)
                    .color(Color32::WHITE),
            );
            ui.add_space(6.0);

            // Build the CPU line graph data.
            // egui_plot expects PlotPoints, which is a list of [x, y] pairs.
            // We enumerate the deque to get (index, value) and map that to [x, y].
            // X = seconds ago from 0 (oldest) to 59 (newest on the right).
            // Web analogy: this is like building a data array for Chart.js:
            //   const data = cpuHistory.map((y, x) => ({ x, y }));
            let cpu_points: PlotPoints = self
                .cpu_history
                .iter()
                .enumerate()
                .map(|(i, &val)| [i as f64, val])
                .collect();

            let cpu_line = Line::new(cpu_points)
                .color(Color32::from_rgb(70, 140, 255)) // blue line
                .width(2.0);

            // Plot widget — this is the chart container.
            // .height() sets the pixel height of the chart area.
            // .include_y() pins the Y axis to always show 0–100.
            // .show_axes() shows the X and Y axis lines.
            // .allow_zoom(false) / .allow_drag(false) = non-interactive (read-only display).
            Plot::new("cpu_plot")
                .height(140.0)
                .include_y(0.0)
                .include_y(100.0)
                .y_axis_label("% Usage")
                .allow_zoom(false)
                .allow_drag(false)
                .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                .show(ui, |plot_ui| {
                    plot_ui.line(cpu_line);
                });

            ui.add_space(16.0);
            // Draw a horizontal rule to visually separate CPU and Memory sections.
            // Like <hr> in HTML.
            ui.separator();
            ui.add_space(16.0);

            // ── MEMORY SECTION ────────────────────────────────────────────────────
            ui.heading(
                egui::RichText::new("Memory Usage")
                    .size(16.0)
                    .color(Color32::from_rgb(100, 220, 130)), // light green
            );
            ui.add_space(4.0);

            // Show both the percentage and the absolute GB figure (like Task Manager).
            ui.label(
                egui::RichText::new(format!(
                    "{:.1}%  ({:.1} GB / {:.1} GB)",
                    current_mem_pct, used_gb, total_gb
                ))
                .size(26.0)
                .color(Color32::WHITE),
            );
            ui.add_space(6.0);

            let mem_points: PlotPoints = self
                .mem_history
                .iter()
                .enumerate()
                .map(|(i, &val)| [i as f64, val])
                .collect();

            let mem_line = Line::new(mem_points)
                .color(Color32::from_rgb(80, 210, 110)) // green line
                .width(2.0);

            Plot::new("mem_plot")
                .height(140.0)
                .include_y(0.0)
                .include_y(100.0)
                .y_axis_label("% Used")
                .allow_zoom(false)
                .allow_drag(false)
                .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                .show(ui, |plot_ui| {
                    plot_ui.line(mem_line);
                });
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
            .with_inner_size([880.0, 620.0])   // Initial window size in logical pixels
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

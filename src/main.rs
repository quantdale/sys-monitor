// ---------------------------------------------------------------------------
// WINDOWS SUBSYSTEM ATTRIBUTE
// ---------------------------------------------------------------------------
// This tells the Windows linker to create a GUI application, not a console app.
// Without it, Windows would open a black cmd.exe terminal behind your window.
// Equivalent to: Project Properties → Linker → System → Subsystem: Windows in MSVC.
// We only apply it on release builds so we can still see panic messages during
// development (cargo run keeps the console open in debug mode).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod platform;
mod metrics;
mod render;

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
        Box::new(|_cc| Box::new(app::SystemMonitor::new()) as Box<dyn eframe::App>),
    )
}

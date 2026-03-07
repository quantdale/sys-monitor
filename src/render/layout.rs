use eframe::egui;
use egui::Color32;

/// Main layout function: renders the full ScrollArea with time-range selector
/// and all metric cards in order (CPU → Memory → Disk(s) → Network → iGPU → dGPU).
pub fn render_layout(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor) {
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
                ui.selectable_value(&mut app.selected_duration, duration, label);
            }
        });

        // Sync the display window size with the (possibly just-changed)
        // selected duration. This runs every frame so graphs immediately
        // reflect any change from the buttons above.
        app.history_length = app.selected_duration as usize;

        // Human-readable X axis label reflecting the selected time range.
        let x_label = match app.selected_duration {
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
        crate::render::cpu::render_cpu(ui, app, x_label);
        ui.add_space(8.0);

        // ── MEMORY CARD ────────────────────────────────────────────────────
        crate::render::memory::render_memory(ui, app, x_label);
        ui.add_space(8.0);

        // ── DISK CARDS (PER PHYSICAL DISK) ──────────────────────────────
        crate::render::disk::render_disk(ui, app, x_label);

        // ── NETWORK CARD ──────────────────────────────────────────────
        crate::render::network::render_network(ui, app, x_label);
        ui.add_space(8.0);

        // ── iGPU + dGPU CARDS ─────────────────────────────────────────
        crate::render::gpu::render_gpu(ui, app, x_label);
        ui.add_space(8.0);
    }); // end ScrollArea
}

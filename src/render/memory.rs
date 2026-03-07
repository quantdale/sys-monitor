use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};

// Memory card rendering extracted from update().
pub fn render_memory(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor, x_label: &str) {
    // Convert raw kilobytes to gigabytes for the human-readable label.
    let used_gb = app.system.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let total_gb = app.system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let current_mem_pct = *app.mem_history.back().unwrap_or(&0.0);

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
        let window = app.history_length.min(app.mem_history.len());
        let skip = app.mem_history.len() - window;
        let mem_points: PlotPoints = app
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
}

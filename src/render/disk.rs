use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};

// Disk cards rendering (per physical disk) extracted from update().
pub fn render_disk(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor, x_label: &str) {
    let disk_palette = [
        Color32::from_rgb(255, 180, 80),
        Color32::from_rgb(255, 220, 80),
        Color32::from_rgb(220, 120, 80),
        Color32::from_rgb(240, 190, 120),
    ];

    for (idx, disk_key) in app.disk_display_order.iter().enumerate() {
        let Some(history) = app.disk_active_histories.get(disk_key) else {
            continue;
        };

        let color = disk_palette[idx % disk_palette.len()];

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(
                egui::RichText::new(format!("Disk {}  —  Active Time", disk_key))
                    .size(16.0)
                    .color(color),
            );
            ui.add_space(4.0);

            let active_now = *history.back().unwrap_or(&0.0);
            ui.label(
                egui::RichText::new(format!("Active Time: {:.1}%", active_now))
                    .size(18.0)
                    .color(color),
            );
            ui.add_space(6.0);

            let window = app.history_length.min(history.len());
            let skip = history.len().saturating_sub(window);
            let points: PlotPoints = history
                .iter()
                .skip(skip)
                .enumerate()
                .map(|(i, &v)| [i as f64, v])
                .collect();

            let line = Line::new(points).color(color).width(2.0).name("Active Time");

            // Y-axis is always 0-100 because % active time is a time fraction,
            // not a throughput value. No rated speed detection is needed.
            // This matches Task Manager's disk active-time graph semantics.
            Plot::new(format!("disk_active_plot_{}", disk_key))
                .height(140.0)
                .include_y(0.0)
                .include_y(100.0)
                .y_axis_label("Active Time %")
                .x_axis_label(x_label)
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .set_margin_fraction(egui::Vec2::new(0.0, 0.05))
                .show(ui, |plot_ui| {
                    plot_ui.line(line);
                });
        });
        ui.add_space(8.0);
    }
}

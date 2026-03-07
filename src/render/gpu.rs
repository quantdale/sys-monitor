use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};

// iGPU and dGPU card rendering extracted from update().
pub fn render_gpu(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor, x_label: &str) {
    // ── iGPU CARD ─────────────────────────────────────────────────
    egui::Frame::group(ui.style()).show(ui, |ui| {
        let igpu_util = *app.igpu_history.back().unwrap_or(&0.0);
        ui.heading(
            egui::RichText::new(format!("GPU  —  Intel iGPU  —  {:.1}%", igpu_util))
                .size(16.0)
                .color(Color32::from_rgb(100, 180, 255)),
        );
        ui.add_space(6.0);

        let window = app.history_length.min(app.igpu_history.len());
        let skip = app.igpu_history.len() - window;
        let igpu_pts: PlotPoints = app.igpu_history
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
        let dgpu_util = *app.dgpu_history.back().unwrap_or(&0.0);
        ui.heading(
            egui::RichText::new(format!("GPU  —  Discrete (NVIDIA/AMD)  —  {:.1}%", dgpu_util))
                .size(16.0)
                .color(Color32::from_rgb(120, 200, 140)),
        );
        ui.add_space(6.0);

        let window = app.history_length.min(app.dgpu_history.len());
        let skip = app.dgpu_history.len() - window;
        let dgpu_pts: PlotPoints = app.dgpu_history
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
}

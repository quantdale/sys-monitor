use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};

// Network card rendering extracted from update().
pub fn render_network(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor, x_label: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.heading(
            egui::RichText::new("Network  —  Total (all adapters)")
                .size(16.0)
                .color(Color32::from_rgb(80, 220, 240)),
        );
        ui.add_space(4.0);

        let net_recv_now = *app.net_recv_history.back().unwrap_or(&0.0);
        let net_sent_now = *app.net_sent_history.back().unwrap_or(&0.0);

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

        let window = app.history_length.min(app.net_recv_history.len());
        let skip_r = app.net_recv_history.len() - window;
        let net_recv_pts: PlotPoints = app.net_recv_history
            .iter().skip(skip_r).enumerate()
            .map(|(i, &v)| [i as f64, v])
            .collect();
        let skip_s = app.net_sent_history.len().saturating_sub(window);
        let net_sent_pts: PlotPoints = app.net_sent_history
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
}

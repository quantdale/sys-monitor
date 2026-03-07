use eframe::egui;
use egui::Color32;
use egui_plot::{Line, Plot, PlotPoints};

// CPU card rendering extracted from update().
pub fn render_cpu(ui: &mut egui::Ui, app: &mut crate::app::SystemMonitor, x_label: &str) {
    // We peek at the most recent value in the deque (the back = newest).
    // back() returns Option<&f64> — None if the deque is empty.
    // unwrap_or(0.0) gives us 0.0 as a safe default before the first reading.
    let current_cpu = *app.cpu_history.back().unwrap_or(&0.0);

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
        let window = app.history_length.min(app.cpu_history.len());
        let skip = app.cpu_history.len() - window;
        let cpu_points: PlotPoints = app
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
}

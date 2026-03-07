// RAM refresh logic extracted from refresh_metrics().
pub fn refresh_memory(app: &mut crate::app::SystemMonitor) {
    // Refresh memory counters. sysinfo calls GlobalMemoryStatusEx() again.
    app.system.refresh_memory();

    // used_memory() / total_memory() both return kilobytes (KB) as u64.
    // We convert to a percentage so the graph Y-axis matches the CPU graph (0–100).
    let used_mem_kb = app.system.used_memory();
    let total_mem_kb = app.system.total_memory();
    let mem_pct = if total_mem_kb > 0 {
        (used_mem_kb as f64 / total_mem_kb as f64) * 100.0
    } else {
        0.0
    };

    app.mem_history.push_back(mem_pct);
    if app.mem_history.len() > app.max_history {
        app.mem_history.pop_front();
    }
}

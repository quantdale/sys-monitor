// CPU refresh logic extracted from refresh_metrics().
pub fn refresh_cpu(app: &mut crate::app::SystemMonitor) {
    // Refresh CPU counters. sysinfo re-queries the PDH counter and computes
    // (new_idle_time - old_idle_time) / elapsed to get a CPU usage percentage.
    app.system.refresh_cpu_usage();

    // global_cpu_usage() returns a f32 representing the average usage % across ALL
    // logical cores. In sysinfo 0.33+, this is a direct f32 value (no .cpu_usage() call).
    // Example: if you have 8 cores and average utilization is 25%, this returns 25.0.
    let cpu_pct = app.system.global_cpu_usage() as f64;

    // Push new values to the BACK of the deque (most recent end).
    app.cpu_history.push_back(cpu_pct);

    // Pop from the front when the buffer exceeds max_history (3600), NOT
    // history_length. We always retain the full hour of data so the user
    // can freely switch between time ranges without losing history.
    if app.cpu_history.len() > app.max_history {
        app.cpu_history.pop_front();
    }
}

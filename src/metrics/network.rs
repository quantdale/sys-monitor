// Network refresh logic extracted from refresh_metrics().
pub fn refresh_network(app: &mut crate::app::SystemMonitor) {
    // ── NETWORK I/O REFRESH ─────────────────────────────────────────────
    // networks.refresh(false) calls GetIfEntry2() for each interface.
    // After this call, received() and transmitted() on each NetworkData
    // return the bytes transferred since the PREVIOUS refresh() — i.e. the
    // delta is pre-computed by sysinfo. At 1 Hz polling, delta == bytes/sec.
    app.networks.refresh(false);

    // Accumulate bytes across all interfaces, skipping:
    //   - Loopback ("lo" on Linux, "Loopback*" on Windows) — localhost traffic
    //     would massively inflate the reading with no useful signal.
    //   - Interfaces with zero traffic this tick — doesn't change the sum but
    //     avoids counting inactive virtual adapters (e.g. VirtualBox, WSL).
    //
    // "Loopback" check: Windows names loopback as "Loopback Pseudo-Interface N"
    // and Linux as "lo". We filter both with a case-insensitive prefix check.
    let mut total_recv_bytes = 0u64;
    let mut total_sent_bytes = 0u64;
    for (iface_name, data) in &app.networks {
        let name_upper = iface_name.to_uppercase();
        if name_upper.contains("LOOPBACK") || name_upper == "LO" {
            continue; // skip loopback
        }
        total_recv_bytes += data.received();
        total_sent_bytes += data.transmitted();
    }

    // Convert bytes/sec → KB/s (divide by 1024).
    // We use KB/s for network because typical usage (streaming, browsing)
    // sits in the 100–10 000 KB/s range — MB/s would make the graph too flat.
    let recv_kbs = total_recv_bytes as f64 / 1024.0;
    let sent_kbs = total_sent_bytes as f64 / 1024.0;

    crate::app::SystemMonitor::push_history(&mut app.net_recv_history, recv_kbs, app.max_history);
    crate::app::SystemMonitor::push_history(&mut app.net_sent_history, sent_kbs, app.max_history);
}

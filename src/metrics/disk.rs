use std::collections::HashMap;
use windows::Win32::System::Performance::{
    PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
};

// PhysicalDisk instance names look like "0 C: D:" (disk index + drive letters).
// One physical disk can have multiple partitions, so one instance may include
// multiple drive letters. If there are no drive letters (system/unformatted
// partition), this returns an empty list and the caller skips that instance.
pub fn pdh_instance_to_drive_letters(instance: &str) -> Vec<String> {
    instance
        .split_whitespace()
        .skip(1) // skip the leading disk index token (e.g. "0")
        .filter(|token| token.ends_with(':'))
        .map(|token| token.to_uppercase())
        .collect()
}

// Read per-instance \PhysicalDisk(*)\% Idle Time values from PDH and invert.
// Returns map of raw PDH instance name -> active time percent (0.0-100.0).
// active% = 100 - idle%  — same value Task Manager's disk graph displays.
pub fn query_disk_active_time(app: &mut crate::app::SystemMonitor) -> HashMap<String, f64> {
    let counter = match app.pdh_disk_active_counter {
        Some(c) => c,
        None => return HashMap::new(),
    };

    // SAFETY: PDH API calls use valid handles and stack-owned output pointers.
    unsafe {
        let mut buffer_size: u32 = 0;
        let mut item_count: u32 = 0;

        // Wildcard counters return multiple instances, so we must use
        // PdhGetFormattedCounterArrayW (not the single-value API).
        // First call probes required buffer size/item count.
        let _ = PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            None,
        );

        if buffer_size == 0 || item_count == 0 {
            return HashMap::new();
        }

        // Second call fills the caller-provided array. The required byte count
        // includes both item structs and trailing instance-name storage, so we
        // must allocate by byte size (not just item_count * struct size).
        // Vec<u64> guarantees 8-byte alignment needed by the embedded f64 union.
        let u64_count = (buffer_size as usize * 3 + 7) / 8;
        let mut backing: Vec<u64> = vec![0u64; u64_count];
        let mut actual_buf_size: u32 = (u64_count * 8) as u32;
        let buf_ptr = backing.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;

        let status = PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut actual_buf_size,
            &mut item_count,
            Some(buf_ptr),
        );

        if status != 0 {
            return HashMap::new();
        }

        let mut result = HashMap::new();
        for i in 0..item_count as usize {
            let item: &PDH_FMT_COUNTERVALUE_ITEM_W = &*buf_ptr.add(i);

            if item.FmtValue.CStatus > 1 {
                continue;
            }

            let name = item.szName.to_string().unwrap_or_default();

            // _Total is the aggregate over all physical disks. We skip it because
            // the UI shows one graph card per physical disk instance.
            if name == "_Total" {
                continue;
            }

            // % Idle Time is the fraction of elapsed time the disk had NO pending I/O.
            // Inverting gives % active time (= % busy), which matches Task Manager's
            // disk graph. PDH can briefly emit out-of-range values near baseline startup;
            // clamp keeps the result physically meaningful (0-100).
            let value = (100.0 - item.FmtValue.Anonymous.doubleValue).clamp(0.0, 100.0);
            result.insert(name, value);
        }

        result
    }
}

// Disk refresh logic: drive-letter mapping loop extracted from refresh_metrics().
// Called only when PdhCollectQueryData has already succeeded (pdh_collected_ok).
pub fn refresh_disk(app: &mut crate::app::SystemMonitor) {
    app.disks.refresh(false);

    // Build drive-letter lookup from sysinfo mount points to map PDH
    // instance labels (e.g. "0 C: D:") onto currently mounted volumes.
    let mut known_drive_letters: HashMap<String, String> = HashMap::new();
    for disk in app.disks.list() {
        let mount = disk.mount_point().to_string_lossy().to_string();
        let mount_upper = mount.to_uppercase();
        if mount_upper.len() >= 2 && mount_upper.as_bytes()[1] == b':' {
            known_drive_letters.insert(mount_upper[..2].to_string(), mount);
        }
    }

    for (instance_name, pct_active) in query_disk_active_time(app) {
        let mapped_letters: Vec<String> = pdh_instance_to_drive_letters(&instance_name)
            .into_iter()
            .filter(|letter| known_drive_letters.contains_key(letter))
            .collect();

        if mapped_letters.is_empty() {
            continue;
        }

        let disk_key = mapped_letters.join(" ");
        if !app.disk_active_histories.contains_key(&disk_key) {
            app.disk_display_order.push(disk_key.clone());
            app.disk_active_histories
                .insert(disk_key.clone(), std::collections::VecDeque::with_capacity(3600));
        }

        if let Some(history) = app.disk_active_histories.get_mut(&disk_key) {
            crate::app::SystemMonitor::push_history(history, pct_active, app.max_history);
        }
    }
}

//! Metrics handlers: GetSiteStats, StreamMetrics.

use crate::proto::{GetSiteStatsRequest, MetricsUpdate, SiteStatsResponse, StreamMetricsRequest};
use crate::validation::{unix_user_for_domain, validate_domain};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_stream::wrappers::IntervalStream;
use tonic::Status;
use tracing::warn;

use super::cgroups::{parse_cpu_stat, read_cgroup_u64};
use super::sites::domain_to_slice_name;

// ── GetSiteStats ──────────────────────────────────────────────────────────────

pub async fn get_site_stats(
    req: GetSiteStatsRequest,
) -> Result<tonic::Response<SiteStatsResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let unix_user = unix_user_for_domain(&domain);
    let slice_name = format!("website-{}.slice", domain_to_slice_name(&domain));
    let cgroup_base = format!("/sys/fs/cgroup/system.slice/{}", slice_name);

    // memory.current
    let memory_current = read_cgroup_u64(&cgroup_base, "memory.current").unwrap_or(0);

    // cpu.stat → usage_usec (cumulative) — convert to ns
    let cpu_usage_usec = parse_cpu_stat(&cgroup_base).unwrap_or(0);
    let cpu_usage_ns   = cpu_usage_usec.saturating_mul(1000);

    // Disk bytes: du -sb /home/<unix_user>
    let disk_bytes = measure_disk_bytes(&format!("/home/{}", unix_user));

    // Request count from access log (last 24h line count)
    let request_count = count_log_requests(&format!("/var/log/nginx/{}.access.log", domain));

    Ok(tonic::Response::new(SiteStatsResponse {
        memory_current,
        cpu_usage_ns,
        disk_bytes,
        net_rx_bytes: 0, // per-site network accounting requires eBPF/tc — not implemented in v1
        net_tx_bytes: 0,
        request_count,
    }))
}

fn measure_disk_bytes(path: &str) -> u64 {
    let output = Command::new("/usr/bin/du")
        .args(["-sb", path])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        }
        Err(e) => {
            warn!(%path, err = %e, "du failed");
            0
        }
    }
}

fn count_log_requests(log_path: &str) -> i64 {
    // Count lines modified in the last 24h — a simple approximation.
    // A production implementation would tail + parse timestamps.
    let output = Command::new("/usr/bin/wc")
        .args(["-l", log_path])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0)
        }
        Err(_) => 0,
    }
}

// ── StreamMetrics ─────────────────────────────────────────────────────────────

pub async fn stream_metrics(
    req: StreamMetricsRequest,
) -> Result<
    tonic::Response<
        std::pin::Pin<
            Box<dyn futures_util::Stream<Item = Result<MetricsUpdate, tonic::Status>> + Send>,
        >,
    >,
    Status,
> {
    // Enforce minimum 1 000 ms interval
    let interval_ms = req.interval_ms.max(1_000) as u64;
    let interval    = Duration::from_millis(interval_ms);

    let stream = {
        let ticker = tokio::time::interval(interval);
        IntervalStream::new(ticker).map(|_| collect_server_metrics())
    };

    Ok(tonic::Response::new(Box::pin(stream)))
}

fn collect_server_metrics() -> Result<MetricsUpdate, tonic::Status> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let (ram_used, ram_total) = read_meminfo();
    let ram_pct = if ram_total > 0 {
        (ram_used as f32 / ram_total as f32) * 100.0
    } else {
        0.0
    };

    let (load_1, load_5, load_15) = read_loadavg();
    let (disk_read_kbps, disk_write_kbps) = read_diskstats();
    let (net_rx_kbps, net_tx_kbps) = read_net_dev();

    // CPU % — approximated from /proc/stat delta
    let cpu_pct = read_cpu_pct();

    Ok(MetricsUpdate {
        timestamp_ms: now_ms,
        cpu_pct,
        ram_pct,
        ram_used_bytes: ram_used,
        ram_total_bytes: ram_total,
        disk_read_kbps,
        disk_write_kbps,
        net_rx_kbps,
        net_tx_kbps,
        load_1,
        load_5,
        load_15,
    })
}

fn read_meminfo() -> (u64, u64) {
    let content = match std::fs::read_to_string("/proc/meminfo") {
        Ok(c)  => c,
        Err(_) => return (0, 0),
    };

    let mut total: u64 = 0;
    let mut available: u64 = 0;

    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = parse_kb_line(line);
        } else if line.starts_with("MemAvailable:") {
            available = parse_kb_line(line);
        }
    }

    let used = total.saturating_sub(available);
    (used * 1024, total * 1024) // convert KB → bytes
}

fn parse_kb_line(line: &str) -> u64 {
    // Format: "MemTotal:       16384000 kB"
    line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn read_loadavg() -> (f32, f32, f32) {
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut parts = content.split_whitespace();
    let l1  = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    let l5  = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    let l15 = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    (l1, l5, l15)
}

/// Read total CPU usage % by comparing a cached previous /proc/stat snapshot
/// against the current one.  No sleep required — the streaming interval
/// (≥1 s) provides the delta window naturally.
fn read_cpu_pct() -> f32 {
    use std::sync::Mutex;
    // Thread-local static pair: (idle, total) from the previous call
    static PREV: Mutex<Option<(u64, u64)>> = Mutex::new(None);

    let snap = read_cpu_stat_snapshot();

    let mut guard = match PREV.lock() {
        Ok(g)  => g,
        Err(_) => return 0.0,
    };

    let result = if let (Some(prev), Some((idle2, total2))) = (*guard, snap) {
        let (idle1, total1) = prev;
        let idle_delta  = idle2.saturating_sub(idle1) as f32;
        let total_delta = total2.saturating_sub(total1) as f32;
        if total_delta > 0.0 {
            (1.0 - idle_delta / total_delta) * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    *guard = snap;
    result
}

fn read_cpu_stat_snapshot() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    for line in content.lines() {
        if line.starts_with("cpu ") {
            let nums: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|s| s.parse::<u64>().ok())
                .collect();
            if nums.len() >= 4 {
                let idle  = nums[3] + nums.get(4).copied().unwrap_or(0); // idle + iowait
                let total: u64 = nums.iter().sum();
                return Some((idle, total));
            }
        }
    }
    None
}

fn read_diskstats() -> (u64, u64) {
    // Read /proc/diskstats, sum sectors read/written for all block devices (non-partitions)
    // and convert to kBps (approximate — single snapshot, not a delta)
    let content = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();

    let mut read_sectors: u64  = 0;
    let mut write_sectors: u64 = 0;

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 14 {
            continue;
        }
        // Only look at whole disks (sd*, nvme*, vd*), skip partitions
        let dev_name = parts[2];
        let is_disk  = (dev_name.starts_with("sd") || dev_name.starts_with("vd") || dev_name.starts_with("nvme"))
            && !dev_name.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);

        if is_disk {
            read_sectors  = read_sectors.saturating_add(parts[5].parse::<u64>().unwrap_or(0));
            write_sectors = write_sectors.saturating_add(parts[9].parse::<u64>().unwrap_or(0));
        }
    }

    // Sectors are 512 bytes; convert to kBps (divide by 2 to get kB)
    (read_sectors / 2, write_sectors / 2)
}

fn read_net_dev() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();

    let mut rx_bytes: u64 = 0;
    let mut tx_bytes: u64 = 0;

    for line in content.lines().skip(2) {
        let trimmed = line.trim();
        if trimmed.starts_with("lo:") {
            continue; // skip loopback
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 10 {
            // Format: interface: rx_bytes rx_packets ... tx_bytes tx_packets ...
            // Index 0 is "iface:", index 1 = rx_bytes, index 9 = tx_bytes
            rx_bytes = rx_bytes.saturating_add(parts[1].parse::<u64>().unwrap_or(0));
            tx_bytes = tx_bytes.saturating_add(parts[9].parse::<u64>().unwrap_or(0));
        }
    }

    (rx_bytes / 1024, tx_bytes / 1024) // bytes → kB
}

// Bring futures_util::StreamExt into scope for the .map() call
use futures_util::StreamExt as _;

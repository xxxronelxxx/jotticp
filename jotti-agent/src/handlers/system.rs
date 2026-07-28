//! System information handlers: GetSystemInfo, GetDiskUsage.

use crate::proto::{DiskUsageResponse, GetDiskUsageRequest, SystemInfoResponse};
use crate::validation::validate_safe_path;
use std::process::Command;
use tonic::Status;
use tracing::warn;

// ── GetSystemInfo ─────────────────────────────────────────────────────────────

pub async fn get_system_info() -> Result<tonic::Response<SystemInfoResponse>, Status> {
    let hostname     = read_hostname();
    let os_version   = read_os_version();
    let kernel       = read_kernel_version();
    let (ram_total, ram_free) = read_meminfo();
    let cpu_count    = read_cpu_count();
    let architecture = read_architecture();
    let uptime_secs  = read_uptime_seconds();

    Ok(tonic::Response::new(SystemInfoResponse {
        hostname,
        os_version,
        kernel,
        ram_total,
        ram_free,
        cpu_count: cpu_count as f32,
        architecture,
        uptime_seconds: uptime_secs,
        orbit_agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}

fn read_hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_os_version() -> String {
    let content = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
            return val.trim_matches('"').to_string();
        }
    }
    "Unknown".to_string()
}

fn read_kernel_version() -> String {
    let output = Command::new("/bin/uname").arg("-r").output();
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_)  => "unknown".to_string(),
    }
}

fn read_meminfo() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total: u64     = 0;
    let mut available: u64 = 0;

    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = parse_kb_value(line);
        } else if line.starts_with("MemAvailable:") {
            available = parse_kb_value(line);
        }
    }

    (total * 1024, available * 1024)
}

fn parse_kb_value(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn read_cpu_count() -> u32 {
    let content = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    content.lines().filter(|l| l.starts_with("processor")).count() as u32
}

fn read_architecture() -> String {
    let output = Command::new("/bin/uname").arg("-m").output();
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_)  => "unknown".to_string(),
    }
}

fn read_uptime_seconds() -> u64 {
    let content = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    content
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

// ── GetDiskUsage ──────────────────────────────────────────────────────────────

pub async fn get_disk_usage(
    req: GetDiskUsageRequest,
) -> Result<tonic::Response<DiskUsageResponse>, Status> {
    let path = if req.path.is_empty() {
        "/".to_string()
    } else {
        validate_safe_path(&req.path)?
    };

    // Use statvfs via /proc/mounts parsing + df for reliable results
    let (total, used, free) = measure_disk(&path)?;

    let used_pct = if total > 0 {
        (used as f32 / total as f32) * 100.0
    } else {
        0.0
    };

    Ok(tonic::Response::new(DiskUsageResponse {
        total_bytes: total,
        used_bytes:  used,
        free_bytes:  free,
        used_pct,
    }))
}

fn measure_disk(path: &str) -> Result<(u64, u64, u64), Status> {
    // df --output=size,used,avail -B1 <path> gives bytes directly
    let output = Command::new("/bin/df")
        .args(["--output=size,used,avail", "-B1", path])
        .output()
        .map_err(|e| Status::internal(format!("df exec failed: {}", e)))?;

    if !output.status.success() {
        return Err(Status::internal(format!(
            "df failed for path: {}",
            path
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Skip header line
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let total = parts[0].parse::<u64>().unwrap_or(0);
            let used  = parts[1].parse::<u64>().unwrap_or(0);
            let free  = parts[2].parse::<u64>().unwrap_or(0);
            return Ok((total, used, free));
        }
    }

    Err(Status::internal("could not parse df output"))
}

//! cgroup v2 resource limit handlers: SetCgroupLimits, GetCgroupStats.

use crate::journal::Journal;
use crate::proto::{CgroupLimitsRequest, CgroupStatsResponse, GetCgroupStatsRequest, StatusResponse};
use crate::validation::validate_domain;
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

use super::sites::domain_to_slice_name;

// ── SetCgroupLimits ───────────────────────────────────────────────────────────

pub async fn set_cgroup_limits(
    req: CgroupLimitsRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let domain = validate_domain(&req.domain)?;

    info!(
        %domain,
        memory_max_bytes = req.memory_max_bytes,
        cpu_quota_pct    = req.cpu_quota_percent,
        pids_max         = req.pids_max,
        "set_cgroup_limits"
    );

    let op_id = journal
        .begin_op(
            "set_cgroup_limits",
            Some(&domain),
            &serde_json::json!({
                "domain": domain,
                "memory_max_bytes": req.memory_max_bytes,
                "cpu_quota_percent": req.cpu_quota_percent,
                "pids_max": req.pids_max,
            }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_set_limits(
        &domain,
        req.memory_max_bytes,
        req.cpu_quota_percent,
        req.pids_max,
        req.io_read_bps_max,
        req.io_write_bps_max,
    );

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

/// Minimum MemoryMax allowed (32 MiB).  Setting anything below this would
/// instantly OOM the site the moment any real process starts inside it.
const MEMORY_MAX_MIN_BYTES: u64 = 32 * 1024 * 1024;

fn do_set_limits(
    domain: &str,
    memory_max_bytes: u64,
    cpu_quota_percent: u32,
    pids_max: u32,
    io_read_bps_max: u64,
    io_write_bps_max: u64,
) -> Result<tonic::Response<StatusResponse>, Status> {
    // Validate: non-zero MemoryMax must be at least 32 MiB.
    // memory_max_bytes == 0 means "unlimited" (mapped to infinity).
    if memory_max_bytes != 0 && memory_max_bytes < MEMORY_MAX_MIN_BYTES {
        return Err(Status::invalid_argument(format!(
            "memory_max_bytes {} is below the minimum of {} bytes (32 MiB). \
             Setting MemoryMax this low would immediately OOM the site.",
            memory_max_bytes, MEMORY_MAX_MIN_BYTES
        )));
    }

    let slice_name = format!("website-{}.slice", domain_to_slice_name(domain));

    // Build the unit file content
    let memory_line = if memory_max_bytes == 0 {
        "MemoryMax=infinity".to_string()
    } else {
        format!("MemoryMax={}", memory_max_bytes)
    };

    let cpu_line = if cpu_quota_percent == 0 {
        "CPUQuota=".to_string() // empty = unlimited
    } else {
        format!("CPUQuota={}%", cpu_quota_percent)
    };

    let tasks_line = if pids_max == 0 {
        "TasksMax=100".to_string()
    } else {
        format!("TasksMax={}", pids_max)
    };

    let mut io_lines = String::new();
    if io_read_bps_max > 0 {
        io_lines.push_str(&format!("IOReadBandwidthMax=/ {}\n", io_read_bps_max));
    }
    if io_write_bps_max > 0 {
        io_lines.push_str(&format!("IOWriteBandwidthMax=/ {}\n", io_write_bps_max));
    }

    let unit_content = format!(
        "[Unit]\n\
         Description=OrbitCP cgroup slice for {domain}\n\n\
         [Slice]\n\
         {memory_line}\n\
         {cpu_line}\n\
         {tasks_line}\n\
         MemorySwapMax=0\n\
         {io_lines}\n\
         [Install]\n\
         WantedBy=slices.target\n",
        domain      = domain,
        memory_line = memory_line,
        cpu_line    = cpu_line,
        tasks_line  = tasks_line,
        io_lines    = io_lines,
    );

    let unit_path = format!("/etc/systemd/system/{}", slice_name);
    std::fs::write(&unit_path, &unit_content)
        .map_err(|e| Status::internal(format!("write slice unit failed: {}", e)))?;

    // daemon-reload to pick up the new unit
    let status = Command::new("/bin/systemctl")
        .args(["daemon-reload"])
        .status()
        .map_err(|e| Status::internal(format!("daemon-reload exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal("systemctl daemon-reload failed"));
    }

    // Apply limits at runtime via set-property (no restart required)
    let mut props: Vec<String> = Vec::new();

    if memory_max_bytes == 0 {
        props.push("MemoryMax=infinity".to_string());
    } else {
        props.push(format!("MemoryMax={}", memory_max_bytes));
    }

    if cpu_quota_percent > 0 {
        // systemd uses thousandths-of-a-percent format for CPUQuotaPerSecUSec
        // but the CLI accepts "N%" directly
        props.push(format!("CPUQuota={}%", cpu_quota_percent));
    }

    if pids_max > 0 {
        props.push(format!("TasksMax={}", pids_max));
    }

    if io_read_bps_max > 0 {
        props.push(format!("IOReadBandwidthMax=/ {}", io_read_bps_max));
    }
    if io_write_bps_max > 0 {
        props.push(format!("IOWriteBandwidthMax=/ {}", io_write_bps_max));
    }

    let mut cmd = Command::new("/bin/systemctl");
    cmd.arg("set-property").arg(&slice_name);
    for prop in &props {
        cmd.arg(prop);
    }

    let status = cmd.status().map_err(|e| Status::internal(format!("set-property exec failed: {}", e)))?;

    if !status.success() {
        // Non-fatal: slice might not be running yet — log but don't fail
        warn!(%slice_name, "systemctl set-property returned non-zero (slice may not be active yet)");
    }

    info!(%domain, %slice_name, "cgroup limits applied");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("cgroup limits applied to {}", slice_name),
    }))
}

// ── GetCgroupStats ────────────────────────────────────────────────────────────

pub async fn get_cgroup_stats(
    req: GetCgroupStatsRequest,
) -> Result<tonic::Response<CgroupStatsResponse>, Status> {
    let domain    = validate_domain(&req.domain)?;
    let slice_name = format!("website-{}.slice", domain_to_slice_name(&domain));

    // cgroup v2 path: /sys/fs/cgroup/system.slice/<slice_name>/
    let cgroup_base = format!("/sys/fs/cgroup/system.slice/{}", slice_name);

    let memory_current = read_cgroup_u64(&cgroup_base, "memory.current").unwrap_or(0);
    let memory_max     = read_cgroup_u64_or_max(&cgroup_base, "memory.max");
    let cpu_usage_usec = parse_cpu_stat(&cgroup_base).unwrap_or(0);
    let pids_current   = read_cgroup_u64(&cgroup_base, "pids.current").unwrap_or(0) as u32;
    let pids_max       = read_cgroup_u64_or_max(&cgroup_base, "pids.max") as u32;

    Ok(tonic::Response::new(CgroupStatsResponse {
        memory_current,
        memory_max,
        cpu_usage_usec,
        pids_current,
        pids_max,
    }))
}

// ── cgroup reading helpers ────────────────────────────────────────────────────

pub fn read_cgroup_u64(base: &str, file: &str) -> Option<u64> {
    let path = format!("{}/{}", base, file);
    let content = std::fs::read_to_string(&path).ok()?;
    content.trim().parse::<u64>().ok()
}

/// Read a cgroup file that may contain "max" (= unlimited).  Returns u64::MAX in that case.
pub fn read_cgroup_u64_or_max(base: &str, file: &str) -> u64 {
    let path    = format!("{}/{}", base, file);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let trimmed = content.trim();
    if trimmed == "max" {
        u64::MAX
    } else {
        trimmed.parse::<u64>().unwrap_or(0)
    }
}

/// Parse `usage_usec` from cpu.stat
pub fn parse_cpu_stat(base: &str) -> Option<u64> {
    let path    = format!("{}/cpu.stat", base);
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("usage_usec ") {
            return rest.trim().parse::<u64>().ok();
        }
    }
    None
}

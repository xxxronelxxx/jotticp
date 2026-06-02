//! Disk quota management (SetDiskQuota — called internally; not in proto v1).
//!
//! Supports both xfs_quota (XFS) and setquota (ext4/other) depending on
//! the filesystem detected at /home.

use crate::validation::validate_unix_user;
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

/// Quota information for a Unix user.
#[derive(Debug)]
pub struct QuotaInfo {
    pub unix_user:    String,
    pub used_kb:      u64,
    pub soft_limit_kb: u64,
    pub hard_limit_kb: u64,
}

/// Set disk quota for a Unix user on /home.
///
/// Tries xfs_quota first (XFS), falls back to setquota (ext4/quota-tools).
/// `soft_kb` and `hard_kb` are in kilobytes (0 = unlimited).
pub fn set_disk_quota(
    unix_user: &str,
    soft_kb: u64,
    hard_kb: u64,
) -> Result<(), Status> {
    let user = validate_unix_user(unix_user)?;

    // Detect filesystem type for /home
    let fs_type = detect_fs_type("/home").unwrap_or_else(|| "ext4".to_string());

    info!(%user, soft_kb, hard_kb, fs_type = %fs_type, "set_disk_quota");

    if fs_type == "xfs" {
        set_xfs_quota(&user, soft_kb, hard_kb)
    } else {
        set_ext4_quota(&user, soft_kb, hard_kb)
    }
}

fn set_xfs_quota(user: &str, soft_kb: u64, hard_kb: u64) -> Result<(), Status> {
    // xfs_quota -x -c "limit -u bsoft=Nk bhard=Nk user" /home
    let limit_spec = format!("limit -u bsoft={}k bhard={}k {}", soft_kb, hard_kb, user);

    let status = Command::new("/usr/sbin/xfs_quota")
        .args(["-x", "-c", &limit_spec, "/home"])
        .status()
        .map_err(|e| Status::internal(format!("xfs_quota exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "xfs_quota failed with exit code {:?}",
            status.code()
        )));
    }

    info!(%user, "XFS quota set");
    Ok(())
}

fn set_ext4_quota(user: &str, soft_kb: u64, hard_kb: u64) -> Result<(), Status> {
    // setquota -u <user> <soft_kb> <hard_kb> <soft_inodes> <hard_inodes> /home
    let status = Command::new("/usr/sbin/setquota")
        .args([
            "-u",
            user,
            &soft_kb.to_string(),
            &hard_kb.to_string(),
            "0",
            "0",
            "/home",
        ])
        .status()
        .map_err(|e| Status::internal(format!("setquota exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "setquota failed with exit code {:?}",
            status.code()
        )));
    }

    info!(%user, "ext4 quota set");
    Ok(())
}

/// Read current quota usage for a user.
pub fn get_quota(unix_user: &str) -> Result<QuotaInfo, Status> {
    let user = validate_unix_user(unix_user)?;

    let output = Command::new("/usr/bin/quota")
        .args(["-u", &user, "--hide-device", "-w"])
        .output()
        .map_err(|e| Status::internal(format!("quota exec failed: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    parse_quota_output(&user, &stdout)
}

/// Parse the output of `quota -u <user>`.
///
/// Typical output:
/// ```
/// Disk quotas for user orbit_example_com (uid 1001):
///      Filesystem  blocks   quota   limit   grace   files   quota   limit   grace
///       /home      102400  512000  512000             456       0       0
/// ```
fn parse_quota_output(user: &str, output: &str) -> Result<QuotaInfo, Status> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('/') || trimmed.starts_with("//") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                let used_kb      = parts[1].parse::<u64>().unwrap_or(0);
                let soft_limit_kb = parts[2].parse::<u64>().unwrap_or(0);
                let hard_limit_kb = parts[3].parse::<u64>().unwrap_or(0);

                return Ok(QuotaInfo {
                    unix_user: user.to_string(),
                    used_kb,
                    soft_limit_kb,
                    hard_limit_kb,
                });
            }
        }
    }

    // If quota is not configured, return zeros
    warn!(%user, "quota output not parseable — returning zeros");
    Ok(QuotaInfo {
        unix_user:     user.to_string(),
        used_kb:       0,
        soft_limit_kb: 0,
        hard_limit_kb: 0,
    })
}

/// Detect the filesystem type for a given mount point by parsing /proc/mounts.
fn detect_fs_type(mount_point: &str) -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;

    // Find the most-specific matching mount (longest path prefix)
    let mut best_match: Option<(usize, String)> = None;

    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let mp      = parts[1];
        let fs_type = parts[2];

        if mount_point.starts_with(mp) {
            let len = mp.len();
            if best_match.as_ref().map_or(true, |(prev_len, _)| len > *prev_len) {
                best_match = Some((len, fs_type.to_string()));
            }
        }
    }

    best_match.map(|(_, fs_type)| fs_type)
}

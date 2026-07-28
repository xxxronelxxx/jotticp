// NOTE: validate_unix_user is also defined in provisioning.rs but is private there.
// We duplicate the regex here to keep module boundaries clean — both must agree.
fn validate_user(user: &str) -> anyhow::Result<()> {
    let re = regex::Regex::new(r"^orbit_[a-z0-9_]{1,24}$").unwrap();
    anyhow::ensure!(re.is_match(user), "Invalid unix user for quota: {}", user);
    Ok(())
}

// ── Filesystem detection ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum FsType {
    Xfs,
    Ext4,
    Btrfs,
    Other(String),
}

/// Parse /proc/mounts to find the filesystem type for the /home mountpoint.
/// Falls back to ext4 if /home is not a separate mount (inherits /).
fn detect_home_fs() -> FsType {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();

    // Prefer the most-specific mount for /home
    let mut best: Option<(usize, &str)> = None; // (mount path length, fs_type)

    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let mount_point = parts[1];
        let fs_type     = parts[2];

        // Check if /home starts with this mount point
        if "/home".starts_with(mount_point) {
            let len = mount_point.len();
            if best.map_or(true, |(bl, _)| len > bl) {
                best = Some((len, fs_type));
            }
        }
    }

    match best.map(|(_, fs)| fs) {
        Some("xfs")   => FsType::Xfs,
        Some("ext4")  => FsType::Ext4,
        Some("btrfs") => FsType::Btrfs,
        Some(other)   => FsType::Other(other.to_string()),
        None          => {
            tracing::warn!("Could not detect /home filesystem type — assuming ext4");
            FsType::Ext4
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Set disk quota for `unix_user` to `quota_mb` megabytes.
///
/// Uses xfs_quota for XFS filesystems and setquota (quota-tools) for ext4.
/// Btrfs subvolume quotas are not supported yet (use btrfs subvolume for that).
pub async fn set_site_quota(unix_user: &str, quota_mb: i64) -> anyhow::Result<()> {
    validate_user(unix_user)?;
    anyhow::ensure!(quota_mb > 0, "quota_mb must be positive, got {}", quota_mb);

    let fs = detect_home_fs();
    tracing::info!(user = %unix_user, quota_mb = %quota_mb, fs = ?fs, "Setting disk quota");

    match fs {
        FsType::Xfs => set_xfs_quota(unix_user, quota_mb).await,
        FsType::Ext4 | FsType::Other(_) => set_ext4_quota(unix_user, quota_mb).await,
        FsType::Btrfs => {
            tracing::warn!(
                user = %unix_user,
                "Btrfs detected — quota not enforced (use subvolume quotas)"
            );
            Ok(())
        }
    }
}

/// Get current disk usage for `unix_user` in megabytes.
///
/// Reads from `quota -u` output, which works for both XFS (repquota) and ext4.
/// Falls back to `du -sm /home/USER` if quota tooling is unavailable.
pub async fn get_site_usage(unix_user: &str) -> anyhow::Result<i64> {
    validate_user(unix_user)?;

    // Try quota -u first (works for ext4 + XFS when quota enabled)
    if let Ok(mb) = read_quota_usage(unix_user) {
        return Ok(mb);
    }

    // Fallback: du -sm
    read_du_usage(unix_user)
}

// ── XFS quota ────────────────────────────────────────────────────────────────

async fn set_xfs_quota(unix_user: &str, quota_mb: i64) -> anyhow::Result<()> {
    // xfs_quota command:
    //   xfs_quota -x -c "limit bsoft=NNNm bhard=NNNm USER" /home
    //
    // We set soft = hard (no grace period warning; hard block is immediate).
    let limit_cmd = format!("limit bsoft={}m bhard={}m {}", quota_mb, quota_mb, unix_user);

    let output = std::process::Command::new("xfs_quota")
        .args(["-x", "-c", &limit_cmd, "/home"])
        .output()
        .map_err(|e| anyhow::anyhow!("xfs_quota exec failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("xfs_quota failed: {}", stderr.trim());
    }

    tracing::debug!(user = %unix_user, quota_mb = %quota_mb, "XFS quota set");
    Ok(())
}

// ── ext4 / quota-tools ───────────────────────────────────────────────────────

async fn set_ext4_quota(unix_user: &str, quota_mb: i64) -> anyhow::Result<()> {
    // setquota -u USER soft_kb hard_kb soft_inodes hard_inodes MOUNT
    //
    // Convert MB → KB for setquota (1 MB = 1024 KB).
    // We set inode limits to 0 (unlimited) — disk space is the constraint we care about.
    let soft_kb = quota_mb * 1024;
    let hard_kb = quota_mb * 1024;

    let output = std::process::Command::new("setquota")
        .args([
            "-u",
            unix_user,
            &soft_kb.to_string(),
            &hard_kb.to_string(),
            "0",   // soft inode limit — unlimited
            "0",   // hard inode limit — unlimited
            "/home",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("setquota exec failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("setquota failed: {}", stderr.trim());
    }

    tracing::debug!(user = %unix_user, quota_mb = %quota_mb, "ext4 quota set");
    Ok(())
}

// ── Usage reading ─────────────────────────────────────────────────────────────

/// Read current usage from `quota -u USER` output (in MB).
fn read_quota_usage(unix_user: &str) -> anyhow::Result<i64> {
    let output = std::process::Command::new("quota")
        .args(["--no-wrap", "-u", unix_user])
        .output()
        .map_err(|e| anyhow::anyhow!("quota exec failed: {}", e))?;

    // quota output format (after header lines):
    //   /dev/sda1  used_kb  soft  hard  grace  files  soft  hard  grace
    //
    // We skip header lines and read the first data line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_quota_output(&stdout, unix_user)
}

fn parse_quota_output(output: &str, unix_user: &str) -> anyhow::Result<i64> {
    // quota --no-wrap emits lines like:
    //   Disk quotas for user orbit_example (uid 1001):
    //        Filesystem  blocks  quota  limit  grace  files  quota  limit  grace
    //   /dev/mapper/home  81920   0     1048576          2048      0      0
    //
    // The blocks column is in 1 KiB units.

    for line in output.lines() {
        let trimmed = line.trim();
        // Skip header lines
        if trimmed.is_empty()
            || trimmed.starts_with("Disk")
            || trimmed.starts_with("Filesystem")
        {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        // First field is the filesystem, second is blocks (KB)
        if let Ok(blocks_kb) = parts[1].parse::<i64>() {
            return Ok(blocks_kb / 1024); // KB → MB
        }
    }

    anyhow::bail!("Could not parse quota output for {}", unix_user)
}

/// Fallback: use `du -sm /home/USER` to measure usage.
fn read_du_usage(unix_user: &str) -> anyhow::Result<i64> {
    let home = format!("/home/{}", unix_user);
    let output = std::process::Command::new("du")
        .args(["-sm", "--", &home])
        .output()
        .map_err(|e| anyhow::anyhow!("du exec failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mb = stdout
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow::anyhow!("Could not parse du output: {}", stdout.trim()))?;

    Ok(mb)
}

// ── Quota alert check ─────────────────────────────────────────────────────────

/// Returns true if the site has used >= 90% of its quota.
pub async fn is_near_quota(unix_user: &str, quota_mb: i64) -> bool {
    if quota_mb <= 0 {
        return false;
    }
    match get_site_usage(unix_user).await {
        Ok(used_mb) => used_mb * 100 / quota_mb >= 90,
        Err(e) => {
            tracing::warn!(user = %unix_user, error = %e, "Quota usage check failed");
            false
        }
    }
}

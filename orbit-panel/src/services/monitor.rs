use crate::AppState;
use std::sync::Arc;
use uuid::Uuid;

// ── Public entry point ────────────────────────────────────────────────────────

/// Collect CPU + memory + disk stats for every active site and write to server_metrics.
///
/// Designed to be called on a fixed interval (e.g. every 30 seconds from orbit-cron).
/// Old metrics (>25h) are pruned on each pass to keep the partitioned table lean.
pub async fn collect_site_stats(state: &Arc<AppState>) -> anyhow::Result<()> {
    let sites = sqlx::query!(
        r#"SELECT s.id, s.domain, s.unix_user, s.server_id
           FROM sites s
           WHERE s.status = 'active' AND s.deleted_at IS NULL"#
    )
    .fetch_all(&state.db)
    .await?;

    tracing::debug!(count = sites.len(), "Collecting site stats");

    let now = chrono::Utc::now();

    for site in &sites {
        let domain    = &site.domain;
        let site_id   = site.id;
        let server_id = site.server_id.unwrap_or(Uuid::nil()); // nil UUID = local server

        // ── Read cgroup v2 stats ───────────────────────────────────────────

        let memory_bytes = read_cgroup_memory(domain).unwrap_or(0);
        let (cpu_usec, cpu_pct) = read_cgroup_cpu(state, site_id, domain).await;

        // ── Disk usage via quota or du ─────────────────────────────────────

        let disk_bytes: i64 = match crate::services::quota::get_site_usage(&site.unix_user).await {
            Ok(mb) => mb * 1024 * 1024,
            Err(_) => 0,
        };

        // ── Write to server_metrics ───────────────────────────────────────

        let result = sqlx::query!(
            r#"INSERT INTO server_metrics
               (id, server_id, site_id, cpu_pct, memory_bytes, disk_bytes, recorded_at)
               VALUES ($1, $2, $3, $4::float8::numeric, $5, $6, $7)"#,
            Uuid::new_v4(),
            server_id,
            site_id,
            cpu_pct as f64,
            memory_bytes as i64,
            disk_bytes,
            now,
        )
        .execute(&state.db)
        .await;

        if let Err(e) = result {
            tracing::warn!(site_id = %site_id, error = %e, "server_metrics insert failed");
        }

        // Store current cpu_usec in Valkey for next-sample delta calculation
        store_cpu_prev(state, site_id, cpu_usec, now.timestamp()).await;
    }

    // ── Prune old metrics (>25 hours) ────────────────────────────────────────

    let pruned = sqlx::query!(
        "DELETE FROM server_metrics WHERE recorded_at < NOW() - INTERVAL '25 hours'"
    )
    .execute(&state.db)
    .await;

    match pruned {
        Ok(r)  => tracing::debug!(rows = r.rows_affected(), "Pruned old server_metrics"),
        Err(e) => tracing::warn!(error = %e, "Failed to prune server_metrics"),
    }

    Ok(())
}

// ── cgroup v2 readers ─────────────────────────────────────────────────────────

/// Validate that a domain is safe to embed in a cgroup filesystem path.
///
/// Allowed: `[a-zA-Z0-9.-]` only — no slashes, no `..`, no whitespace.
/// Sites from the DB already pass provisioning validation, but we re-check here
/// because monitor reads every active site and a tampered DB row could otherwise
/// cause path traversal into `/sys/fs/cgroup/`.
fn cgroup_domain_safe(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !domain.contains("..")
}

/// Read memory.current (bytes) from the site's cgroup v2 slice.
/// Returns `None` if the cgroup doesn't exist (site not yet provisioned) or on any error.
fn read_cgroup_memory(domain: &str) -> Option<u64> {
    // Guard: reject domains that could traverse the cgroup path.
    if !cgroup_domain_safe(domain) {
        tracing::warn!(domain = %domain, "read_cgroup_memory: domain failed safety check — skipping");
        return None;
    }

    // cgroup v2 unified hierarchy
    let path = format!(
        "/sys/fs/cgroup/system.slice/website-{}.slice/memory.current",
        domain
    );

    match std::fs::read_to_string(&path) {
        Ok(s) => s.trim().parse::<u64>().ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Site cgroup doesn't exist yet (still provisioning or never started).
            // This is normal — return 0 silently.
            None
        }
        Err(e) => {
            tracing::warn!(domain = %domain, error = %e, "read_cgroup_memory: unexpected error");
            None
        }
    }
}

/// Read cpu.stat usage_usec and compute CPU% vs the previous sample stored in Valkey.
///
/// Returns (current_usec, cpu_pct). `cpu_pct` is 0.0 if no previous sample exists yet
/// or if the cgroup doesn't exist (ENOENT — site not yet provisioned).
async fn read_cgroup_cpu(
    state: &Arc<AppState>,
    site_id: Uuid,
    domain: &str,
) -> (u64, f32) {
    // Guard: reject domains that could traverse the cgroup path.
    if !cgroup_domain_safe(domain) {
        tracing::warn!(domain = %domain, "read_cgroup_cpu: domain failed safety check — skipping");
        return (0, 0.0);
    }

    let path = format!(
        "/sys/fs/cgroup/system.slice/website-{}.slice/cpu.stat",
        domain
    );

    let contents = match std::fs::read_to_string(&path) {
        Ok(c)  => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Site cgroup not provisioned yet — return zeros silently.
            return (0, 0.0);
        }
        Err(e) => {
            tracing::warn!(domain = %domain, error = %e, "read_cgroup_cpu: unexpected error");
            return (0, 0.0);
        }
    };

    // cpu.stat format:
    //   usage_usec NNN
    //   user_usec NNN
    //   system_usec NNN
    let usage_usec = contents
        .lines()
        .find(|l| l.starts_with("usage_usec "))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    // Retrieve previous sample from Valkey
    let prev_key = format!("orbit:cpu_prev:{}", site_id);
    let prev_data: Option<String> = {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        conn.get(&prev_key).await.unwrap_or(None)
    };

    let cpu_pct = if let Some(json) = prev_data {
        let prev: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        let prev_usec = prev["usec"].as_u64().unwrap_or(0);
        let prev_ts   = prev["ts"].as_i64().unwrap_or(0);

        let now_ts = chrono::Utc::now().timestamp();
        let elapsed_us = (now_ts - prev_ts).max(1) as u64 * 1_000_000; // seconds → microseconds

        let delta_usec = usage_usec.saturating_sub(prev_usec);
        let pct = (delta_usec as f64 / elapsed_us as f64 * 100.0) as f32;
        pct.clamp(0.0, 100.0)
    } else {
        0.0
    };

    (usage_usec, cpu_pct)
}

// ── Valkey CPU sample store ───────────────────────────────────────────────────

async fn store_cpu_prev(
    state:    &Arc<AppState>,
    site_id:  Uuid,
    usec:     u64,
    now_ts:   i64,
) {
    use redis::AsyncCommands;
    let key = format!("orbit:cpu_prev:{}", site_id);
    let val = serde_json::json!({ "usec": usec, "ts": now_ts }).to_string();
    let mut conn = state.valkey.clone();
    // TTL: 2× the expected collection interval (2 × 30s = 60s).
    // If the agent misses a sample the stale key expires and we reset cleanly.
    let _: () = conn.set_ex(key, val, 60).await.unwrap_or(());
}

// ── Per-site stat retrieval (for WebSocket dashboard feed) ───────────────────

/// Return the most recent metrics row for a given site.
pub async fn latest_site_stats(
    state:   &Arc<AppState>,
    site_id: Uuid,
) -> anyhow::Result<Option<SiteMetrics>> {
    let row = sqlx::query!(
        r#"SELECT cpu_pct::float8 AS cpu_pct, memory_bytes, disk_bytes, recorded_at
           FROM server_metrics
           WHERE site_id = $1
           ORDER BY recorded_at DESC
           LIMIT 1"#,
        site_id
    )
    .fetch_optional(&state.db)
    .await?;

    Ok(row.map(|r| SiteMetrics {
        site_id,
        cpu_pct:      r.cpu_pct.unwrap_or(0.0) as f32,
        memory_bytes: r.memory_bytes.unwrap_or(0) as u64,
        disk_bytes:   r.disk_bytes.unwrap_or(0) as u64,
        recorded_at:  r.recorded_at,
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct SiteMetrics {
    pub site_id:      Uuid,
    pub cpu_pct:      f32,
    pub memory_bytes: u64,
    pub disk_bytes:   u64,
    pub recorded_at:  chrono::DateTime<chrono::Utc>,
}

/// Return the last N metrics rows for a given site (for sparkline graphs).
pub async fn site_metrics_history(
    state:   &Arc<AppState>,
    site_id: Uuid,
    hours:   i64,
) -> anyhow::Result<Vec<SiteMetrics>> {
    let rows = sqlx::query!(
        r#"SELECT cpu_pct::float8 AS cpu_pct, memory_bytes, disk_bytes, recorded_at
           FROM server_metrics
           WHERE site_id = $1
             AND recorded_at > NOW() - ($2::bigint * INTERVAL '1 hour')
           ORDER BY recorded_at ASC"#,
        site_id,
        hours,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(rows.into_iter().map(|r| SiteMetrics {
        site_id,
        cpu_pct:      r.cpu_pct.unwrap_or(0.0) as f32,
        memory_bytes: r.memory_bytes.unwrap_or(0) as u64,
        disk_bytes:   r.disk_bytes.unwrap_or(0) as u64,
        recorded_at:  r.recorded_at,
    }).collect())
}

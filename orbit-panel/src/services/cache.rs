use crate::AppState;
use std::sync::Arc;
use uuid::Uuid;

// ── OPcache via PHP-CLI ───────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct OpcacheStats {
    pub enabled:          bool,
    pub hit_rate:         f64,
    pub memory_used_mb:   f64,
    pub memory_free_mb:   f64,
    pub cached_scripts:   i64,
    pub jit_enabled:      bool,
    pub jit_buffer_size:  i64,
}

/// Flush OPcache for a specific site's PHP-FPM pool via UNIX socket.
///
/// This is done by running a tiny PHP one-liner as the site's unix user
/// to hit `opcache_reset()` — the PHP binary is in the allowlist.
pub async fn flush_opcache(php_version: &str, unix_user: &str) -> anyhow::Result<()> {
    let php_bin = format!("/usr/bin/php{}", php_version);

    let status = tokio::task::spawn_blocking({
        let php_bin  = php_bin.clone();
        let unix_user = unix_user.to_string();
        move || {
            std::process::Command::new("/usr/bin/sudo")
                .args(["-u", &unix_user, &php_bin, "-r", "opcache_reset();"])
                .status()
        }
    })
    .await??;

    anyhow::ensure!(status.success(), "opcache_reset() failed (exit {:?})", status.code());
    tracing::info!(%unix_user, %php_version, "OPcache flushed");
    Ok(())
}

/// Get OPcache statistics by running PHP-CLI as the site unix user.
pub async fn get_opcache_stats(php_version: &str, unix_user: &str) -> anyhow::Result<OpcacheStats> {
    let php_bin = format!("/usr/bin/php{}", php_version);
    let script = r#"
        $s = opcache_get_status(false);
        $c = opcache_get_configuration();
        if (!$s) { echo json_encode(["enabled"=>false]); exit; }
        $mem = $s['memory_usage'];
        $total = $mem['used_memory'] + $mem['free_memory'] + $mem['wasted_memory'];
        echo json_encode([
            'enabled'         => true,
            'hit_rate'        => $s['opcache_statistics']['opcache_hit_rate'] ?? 0,
            'memory_used_mb'  => round($mem['used_memory'] / 1048576, 2),
            'memory_free_mb'  => round($mem['free_memory'] / 1048576, 2),
            'cached_scripts'  => $s['opcache_statistics']['num_cached_scripts'] ?? 0,
            'jit_enabled'     => !empty($s['jit']['enabled']),
            'jit_buffer_size' => $c['directives']['opcache.jit_buffer_size'] ?? 0,
        ]);
    "#;

    let output = tokio::task::spawn_blocking({
        let php_bin  = php_bin.clone();
        let unix_user = unix_user.to_string();
        let script    = script.to_string();
        move || {
            std::process::Command::new("/usr/bin/sudo")
                .args(["-u", &unix_user, &php_bin, "-r", &script])
                .output()
        }
    })
    .await??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| anyhow::anyhow!("Failed to parse opcache stats JSON: {}: {}", e, stdout))?;

    Ok(OpcacheStats {
        enabled:         v["enabled"].as_bool().unwrap_or(false),
        hit_rate:        v["hit_rate"].as_f64().unwrap_or(0.0),
        memory_used_mb:  v["memory_used_mb"].as_f64().unwrap_or(0.0),
        memory_free_mb:  v["memory_free_mb"].as_f64().unwrap_or(0.0),
        cached_scripts:  v["cached_scripts"].as_i64().unwrap_or(0),
        jit_enabled:     v["jit_enabled"].as_bool().unwrap_or(false),
        jit_buffer_size: v["jit_buffer_size"].as_i64().unwrap_or(0),
    })
}

// ── Valkey per-site flush ─────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct ValkeyStats {
    pub connected:         bool,
    pub memory_used_mb:    f64,
    pub memory_peak_mb:    f64,
    pub hit_rate:          f64,
    pub connected_clients: i64,
    pub uptime_seconds:    i64,
    pub version:           String,
}

/// Flush all Valkey keys matching `site_id:*` pattern.
/// Uses SCAN to avoid blocking the server (no FLUSHALL/KEYS).
pub async fn flush_site_valkey_keys(state: &Arc<AppState>, site_id: Uuid) -> anyhow::Result<u64> {
    use redis::AsyncCommands;

    let pattern = format!("site:{}:*", site_id);
    let mut conn = state.valkey.clone();
    let mut cursor: u64 = 0;
    let mut deleted: u64 = 0;

    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(&pattern)
            .arg("COUNT")
            .arg(100u32)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("SCAN error: {}", e))?;

        if !keys.is_empty() {
            let count = keys.len() as u64;
            let _: () = conn.del(keys).await
                .map_err(|e| anyhow::anyhow!("DEL error: {}", e))?;
            deleted += count;
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    tracing::info!(site_id = %site_id, deleted = deleted, "Flushed Valkey keys for site");
    Ok(deleted)
}

/// Get global Valkey server statistics (for admin dashboard).
pub async fn get_valkey_stats(state: &Arc<AppState>) -> anyhow::Result<ValkeyStats> {
    let mut conn = state.valkey.clone();
    let info: String = redis::cmd("INFO")
        .arg("all")
        .query_async(&mut conn)
        .await
        .map_err(|e| anyhow::anyhow!("INFO command failed: {}", e))?;

    let mut stats = ValkeyStats {
        connected:         true,
        memory_used_mb:    0.0,
        memory_peak_mb:    0.0,
        hit_rate:          0.0,
        connected_clients: 0,
        uptime_seconds:    0,
        version:           "unknown".into(),
    };

    let mut hits: i64 = 0;
    let mut misses: i64 = 0;

    for line in info.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        let (k, v) = (parts[0].trim(), parts[1].trim());
        match k {
            "redis_version"                   => stats.version           = v.into(),
            "used_memory"                     => stats.memory_used_mb    = parse_f64(v) / 1_048_576.0,
            "used_memory_peak"                => stats.memory_peak_mb    = parse_f64(v) / 1_048_576.0,
            "connected_clients"               => stats.connected_clients = parse_i64(v),
            "uptime_in_seconds"               => stats.uptime_seconds    = parse_i64(v),
            "keyspace_hits"                   => hits                    = parse_i64(v),
            "keyspace_misses"                 => misses                  = parse_i64(v),
            _ => {}
        }
    }

    let total = hits + misses;
    stats.hit_rate = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };

    Ok(stats)
}

// ── Browser cache header presets ──────────────────────────────────────────────

/// Valid preset values for cache-control header configuration.
pub fn validate_cache_preset(preset: &str) -> anyhow::Result<()> {
    match preset {
        "aggressive" | "moderate" | "disabled" => Ok(()),
        _ => Err(anyhow::anyhow!("preset must be: aggressive, moderate, or disabled")),
    }
}

/// Apply browser cache preset to the site's web server configuration.
/// Writes a cache.conf snippet to the nginx/OLS include directory.
pub async fn apply_cache_preset(domain: &str, preset: &str) -> anyhow::Result<()> {
    validate_cache_preset(preset)?;

    let config_content = match preset {
        "aggressive" => concat!(
            "# aggressive: 1-year cache for assets, no-store for HTML\n",
            "location ~* \\.(js|css|png|jpg|jpeg|gif|svg|woff|woff2|ttf|ico)$ {\n",
            "    add_header Cache-Control \"public, max-age=31536000, immutable\";\n",
            "    expires 1y;\n",
            "}\n",
            "location ~* \\.html?$ {\n",
            "    add_header Cache-Control \"no-store, no-cache, must-revalidate\";\n",
            "    expires 0;\n",
            "}\n",
        ),
        "moderate" => concat!(
            "# moderate: 1-week cache for assets, 5-min for HTML\n",
            "location ~* \\.(js|css|png|jpg|jpeg|gif|svg|woff|woff2|ttf|ico)$ {\n",
            "    add_header Cache-Control \"public, max-age=604800\";\n",
            "    expires 7d;\n",
            "}\n",
            "location ~* \\.html?$ {\n",
            "    add_header Cache-Control \"public, max-age=300\";\n",
            "    expires 5m;\n",
            "}\n",
        ),
        _ => "# disabled: no caching\nadd_header Cache-Control \"no-store, no-cache, must-revalidate\";\nexpires 0;\n",
    };

    let conf_path = format!("/etc/nginx/cache-presets/{}.conf", sanitise_domain(domain));

    // Atomic write: write to tmp then rename
    let tmp_path = format!("{}.tmp", conf_path);
    tokio::fs::create_dir_all("/etc/nginx/cache-presets/").await?;
    tokio::fs::write(&tmp_path, config_content).await?;
    tokio::fs::rename(&tmp_path, &conf_path).await?;

    tracing::info!(%domain, %preset, "Applied cache preset");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_f64(s: &str) -> f64 { s.parse::<f64>().unwrap_or(0.0) }
fn parse_i64(s: &str) -> i64 { s.parse::<i64>().unwrap_or(0) }

fn sanitise_domain(domain: &str) -> String {
    domain.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

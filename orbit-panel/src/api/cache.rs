use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OpcacheStats {
    pub enabled:         bool,
    pub hit_rate:        f64,
    pub memory_used_mb:  f64,
    pub memory_free_mb:  f64,
    pub cached_scripts:  i64,
    pub jit_enabled:     bool,
    pub jit_buffer_size: i64,
}

#[derive(Debug, Serialize)]
pub struct ValkeyStats {
    pub connected:        bool,
    pub keys:             i64,
    pub memory_used_mb:   f64,
    pub memory_peak_mb:   f64,
    pub hit_rate:         f64,
    pub connected_clients: i64,
    pub uptime_seconds:   i64,
    pub version:          String,
}

#[derive(Debug, Serialize)]
pub struct ValkeyMonitorEntry {
    pub timestamp:  f64,
    pub command:    String,
}

#[derive(Debug, Deserialize)]
pub struct CacheHeadersRequest {
    pub preset: String, // "aggressive" | "moderate" | "disabled"
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Global stats (no site_id — dashboard overview)
        .route("/api/v1/cache/opcache",                    get(get_opcache_stats_global))
        .route("/api/v1/cache/valkey",                     get(get_valkey_stats_global))
        // Per-site stats
        .route("/api/v1/cache/opcache/{site_id}",          get(get_opcache_stats))
        .route("/api/v1/cache/opcache/{site_id}/flush",    post(flush_opcache))
        .route("/api/v1/cache/valkey/{site_id}",           get(get_valkey_stats))
        .route("/api/v1/cache/valkey/{site_id}/flush",     post(flush_valkey))
        .route("/api/v1/cache/valkey/{site_id}/monitor",   get(valkey_monitor))
        .route("/api/v1/cache/headers/{site_id}",          put(set_cache_headers))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn fetch_site_checked(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<String> {
    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;
    Ok(site.unix_user)
}

// ── Global (no site_id) stats ─────────────────────────────────────────────────

async fn get_opcache_stats_global(
    claims: Claims,
) -> ApiResult<Json<OpcacheStats>> {
    let _user_id: uuid::Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    Ok(Json(read_opcache_stats("__global__")))
}

async fn get_valkey_stats_global(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<ValkeyStats>> {
    let _user_id: uuid::Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    // Connect to the main Valkey instance using the app's existing connection
    let valkey_url = std::env::var("VALKEY_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let stats = match redis::Client::open(valkey_url.as_str()) {
        Ok(client) => match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                let info: String = redis::cmd("INFO").query_async(&mut conn).await.unwrap_or_default();
                if info.is_empty() { valkey_stats_disconnected() } else { parse_valkey_info(&info) }
            }
            Err(_) => valkey_stats_disconnected(),
        },
        Err(_) => valkey_stats_disconnected(),
    };
    drop(state);
    Ok(Json(stats))
}

// ── OPcache handlers ──────────────────────────────────────────────────────────

async fn get_opcache_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<OpcacheStats>> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    let stats = read_opcache_stats(&unix_user);
    Ok(Json(stats))
}

async fn flush_opcache(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({ "type": "flush_opcache", "site_id": site_id });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Valkey handlers ───────────────────────────────────────────────────────────

async fn get_valkey_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<ValkeyStats>> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    // Each site's Valkey instance listens on a Unix socket
    let socket_path = format!("/run/orbit/valkey/{}.sock", unix_user);

    let stats = connect_and_info(&socket_path).await;
    Ok(Json(stats))
}

async fn connect_and_info(socket_path: &str) -> ValkeyStats {
    let url = format!("redis+unix://{}", socket_path);
    let client = match redis::Client::open(url.as_str()) {
        Ok(c) => c,
        Err(_) => return valkey_stats_disconnected(),
    };
    let mut conn = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(_) => return valkey_stats_disconnected(),
    };

    let info: String = match redis::cmd("INFO").query_async(&mut conn).await {
        Ok(v) => v,
        Err(_) => return valkey_stats_disconnected(),
    };

    parse_valkey_info(&info)
}

fn parse_valkey_info(info: &str) -> ValkeyStats {
    let mut keys: i64 = 0;
    let mut mem_used: f64 = 0.0;
    let mut mem_peak: f64 = 0.0;
    let mut hits: i64 = 0;
    let mut misses: i64 = 0;
    let mut clients: i64 = 0;
    let mut uptime: i64 = 0;
    let mut version = String::from("unknown");

    for line in info.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 { continue; }
        let (k, v) = (parts[0].trim(), parts[1].trim());
        match k {
            "redis_version"           => version = v.to_string(),
            "uptime_in_seconds"       => uptime = v.parse().unwrap_or(0),
            "connected_clients"       => clients = v.parse().unwrap_or(0),
            "used_memory"             => mem_used = v.parse::<f64>().unwrap_or(0.0) / 1024.0 / 1024.0,
            "used_memory_peak"        => mem_peak = v.parse::<f64>().unwrap_or(0.0) / 1024.0 / 1024.0,
            "keyspace_hits"           => hits = v.parse().unwrap_or(0),
            "keyspace_misses"         => misses = v.parse().unwrap_or(0),
            k if k.starts_with("db") => {
                // db0:keys=42,expires=0,avg_ttl=0
                if let Some(kv) = v.split(',').next() {
                    if let Some(n) = kv.strip_prefix("keys=") {
                        keys += n.parse::<i64>().unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }

    let total = hits + misses;
    let hit_rate = if total > 0 { hits as f64 / total as f64 * 100.0 } else { 0.0 };

    ValkeyStats {
        connected:         true,
        keys,
        memory_used_mb:    mem_used,
        memory_peak_mb:    mem_peak,
        hit_rate,
        connected_clients: clients,
        uptime_seconds:    uptime,
        version,
    }
}

fn valkey_stats_disconnected() -> ValkeyStats {
    ValkeyStats {
        connected:         false,
        keys:              0,
        memory_used_mb:    0.0,
        memory_peak_mb:    0.0,
        hit_rate:          0.0,
        connected_clients: 0,
        uptime_seconds:    0,
        version:           "unknown".into(),
    }
}

async fn flush_valkey(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    let socket_path = format!("/run/orbit/valkey/{}.sock", unix_user);
    let url = format!("redis+unix://{}", socket_path);

    let client = redis::Client::open(url.as_str())
        .map_err(|e| ApiError::ExternalService(format!("Valkey not reachable: {}", e)))?;
    let mut conn = client.get_multiplexed_async_connection().await
        .map_err(|e| ApiError::ExternalService(format!("Valkey connect failed: {}", e)))?;

    let _: () = redis::cmd("FLUSHDB")
        .query_async(&mut conn)
        .await
        .map_err(|e| ApiError::ExternalService(format!("FLUSHDB failed: {}", e)))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn valkey_monitor(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ValkeyMonitorEntry>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    let socket_path = format!("/run/orbit/valkey/{}.sock", unix_user);
    let url = format!("redis+unix://{}", socket_path);

    let client = redis::Client::open(url.as_str())
        .map_err(|e| ApiError::ExternalService(format!("Valkey not reachable: {}", e)))?;

    // Use MONITOR with a 5-second timeout, collect up to 20 commands
    let mut conn = client.get_connection()
        .map_err(|e| ApiError::ExternalService(format!("Valkey connect failed: {}", e)))?;

    let entries = collect_monitor_entries(&mut conn, 5, 20);
    Ok(Json(entries))
}

fn collect_monitor_entries(conn: &mut redis::Connection, _timeout_secs: u64, max_entries: usize) -> Vec<ValkeyMonitorEntry> {

    // SLOWLOG GET is used as a proxy for recent command history.
    // True MONITOR mode is available via redis-cli; SLOWLOG is safer for panel display.
    conn.set_read_timeout(Some(std::time::Duration::from_millis(200))).ok();
    let mut entries = Vec::new();

    // Read last 20 entries from SLOWLOG as a proxy for recent command history
    let slowlog: redis::RedisResult<Vec<Vec<redis::Value>>> =
        redis::cmd("SLOWLOG").arg("GET").arg(max_entries as i64).query(conn);

    if let Ok(entries_raw) = slowlog {
        for entry in entries_raw.into_iter().take(max_entries) {
            // SLOWLOG format: [id, timestamp, duration_us, [cmd args...], ...]
            if entry.len() >= 4 {
                let ts: f64 = match &entry[1] {
                    redis::Value::Int(t) => *t as f64,
                    _ => 0.0,
                };
                let cmd = match &entry[3] {
                    // redis 0.25: arrays are Value::Bulk(Vec<Value>), bulk strings are Value::Data(Vec<u8>)
                    redis::Value::Bulk(parts) => {
                        parts.iter().map(|v| match v {
                            redis::Value::Data(b) => String::from_utf8_lossy(b).to_string(),
                            redis::Value::Status(s) => s.clone(),
                            _ => String::new(),
                        }).collect::<Vec<_>>().join(" ")
                    }
                    _ => String::new(),
                };
                if !cmd.is_empty() {
                    entries.push(ValkeyMonitorEntry { timestamp: ts, command: cmd });
                }
            }
        }
    }

    entries
}

// ── Browser cache header preset handler ──────────────────────────────────────

async fn set_cache_headers(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<CacheHeadersRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_checked(&state, site_id, user_id, is_admin).await?;

    let preset = req.preset.as_str();
    let valid_presets = ["aggressive", "moderate", "disabled"];
    if !valid_presets.contains(&preset) {
        return Err(ApiError::Validation(format!(
            "preset must be one of: {}",
            valid_presets.join(", ")
        )));
    }

    // Persist setting in DB
    sqlx::query!(
        "INSERT INTO site_settings (site_id, setting_key, setting_value)
         VALUES ($1, 'cache_headers_preset', $2)
         ON CONFLICT (site_id, setting_key) DO UPDATE SET setting_value = $2",
        site_id, preset
    )
    .execute(&state.db)
    .await?;

    // Queue vhost reload with new cache headers
    let (static_ttl, html_cache_control) = match preset {
        "aggressive" => ("31536000", "no-cache, no-store, must-revalidate"),
        "moderate"   => ("86400", "no-cache"),
        "disabled"   => ("0", "no-store"),
        _            => unreachable!(),
    };

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "update_cache_headers",
            "site_id": site_id,
            "preset": preset,
            "static_asset_ttl": static_ttl,
            "html_cache_control": html_cache_control,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_opcache_stats(unix_user: &str) -> OpcacheStats {
    let stats_path = format!("/run/orbit/opcache/{}.json", unix_user);
    if let Ok(content) = std::fs::read_to_string(&stats_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            return OpcacheStats {
                enabled:         v["enabled"].as_bool().unwrap_or(false),
                hit_rate:        v["hit_rate"].as_f64().unwrap_or(0.0),
                memory_used_mb:  v["memory_used_mb"].as_f64().unwrap_or(0.0),
                memory_free_mb:  v["memory_free_mb"].as_f64().unwrap_or(0.0),
                cached_scripts:  v["cached_scripts"].as_i64().unwrap_or(0),
                jit_enabled:     v["jit_enabled"].as_bool().unwrap_or(false),
                jit_buffer_size: v["jit_buffer_size"].as_i64().unwrap_or(0),
            };
        }
    }
    OpcacheStats {
        enabled: false, hit_rate: 0.0, memory_used_mb: 0.0,
        memory_free_mb: 0.0, cached_scripts: 0, jit_enabled: false, jit_buffer_size: 0,
    }
}

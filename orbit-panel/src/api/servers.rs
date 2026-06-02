use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateServerRequest {
    pub label:      String,
    pub hostname:   String,
    pub ip_address: String,
    pub ssh_port:   Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServerRequest {
    pub label:    Option<String>,
    pub ssh_port: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ServerResponse {
    pub id:           Uuid,
    pub label:        String,
    pub ip:           String,
    pub ssh_port:     i32,
    pub status:       String,
    pub agent_version: Option<String>,
    pub os_info:      Option<String>,
    pub enrolled_at:  Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at:   DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ServerMetricsResponse {
    pub server_id:     Uuid,
    pub cpu_pct:       f64,
    pub ram_used_mb:   i64,
    pub ram_total_mb:  i64,
    pub disk_used_gb:  f64,
    pub disk_total_gb: f64,
    pub net_in_mbps:   f64,
    pub net_out_mbps:  f64,
    pub load_avg_1m:   f64,
    pub recorded_at:   DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct EnrollTokenResponse {
    pub token:      String,
    pub server_id:  Uuid,
    pub expires_in: u64,  // seconds
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/servers",                  get(list_servers).post(create_server))
        .route("/api/v1/servers/{id}",              get(get_server).put(update_server).delete(delete_server))
        .route("/api/v1/servers/{id}/enroll",       post(enroll_server))
        .route("/api/v1/servers/{id}/metrics",      get(get_metrics))
        .route("/api/v1/servers/{id}/test",         post(test_connectivity))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

fn require_admin(claims: &Claims) -> ApiResult<Uuid> {
    let (user_id, is_admin) = caller(claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Server management requires admin role".into()));
    }
    Ok(user_id)
}

/// True if `ip` belongs to this (panel) host — loopback or any of the host's own
/// interface addresses. Used so the local server is managed directly, no agent.
fn is_local_ip(ip: &str) -> bool {
    if ip.is_empty() || ip == "127.0.0.1" || ip == "::1" || ip == "localhost" {
        return true;
    }
    if let Ok(out) = std::process::Command::new("hostname").arg("-I").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        return s.split_whitespace().any(|local| local == ip);
    }
    false
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_servers(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ServerResponse>>> {
    require_admin(&claims)?;

    let rows = sqlx::query!(
        "SELECT id, label, ip::text as ip_text, ssh_port, status,
                agent_version, last_seen_at, created_at
         FROM servers WHERE deleted_at IS NULL
         ORDER BY created_at DESC"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ServerResponse {
        id:            r.id,
        label:         r.label,
        ip:            r.ip_text.unwrap_or_default(),
        ssh_port:      r.ssh_port,
        status:        r.status,
        agent_version: r.agent_version,
        os_info:       None,
        enrolled_at:   None,
        last_seen_at:  r.last_seen_at,
        created_at:    r.created_at,
    }).collect()))
}

#[derive(Debug, Serialize)]
pub struct ServerDetailResponse {
    pub id:            Uuid,
    pub label:         String,
    pub hostname:      String,
    pub ip:            String,
    pub ssh_port:      i32,
    pub status:        String,
    pub agent_version: Option<String>,
    pub os_version:    Option<String>,
    pub cpu_count:     Option<i32>,
    pub ram_total_mb:  Option<i64>,
    pub disk_total_gb: Option<i64>,
    pub cpu_pct:       Option<f64>,
    pub ram_pct:       Option<f64>,
    pub disk_pct:      Option<f64>,
    pub load_1:        Option<f64>,
    pub uptime_seconds: Option<i64>,
    pub last_seen_at:  Option<DateTime<Utc>>,
    pub created_at:    DateTime<Utc>,
}

async fn get_server(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ServerDetailResponse>> {
    require_admin(&claims)?;

    let row = sqlx::query!(
        "SELECT id, label, hostname, ip::text as ip_text, host(ip) as ip_host, ssh_port, status,
                agent_version, os_version, cpu_count, ram_total_mb, disk_total_gb,
                last_seen_at, created_at
         FROM servers WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "server" })?;

    let mut resp = ServerDetailResponse {
        id:            row.id,
        label:         row.label,
        hostname:      row.hostname,
        ip:            row.ip_text.unwrap_or_default(),
        ssh_port:      row.ssh_port,
        status:        row.status,
        agent_version: row.agent_version,
        os_version:    row.os_version,
        cpu_count:     row.cpu_count,
        ram_total_mb:  row.ram_total_mb,
        disk_total_gb: row.disk_total_gb,
        cpu_pct:       None,
        ram_pct:       None,
        disk_pct:      None,
        load_1:        None,
        uptime_seconds: None,
        last_seen_at:  row.last_seen_at,
        created_at:    row.created_at,
    };

    // Local (panel) host: fill live metrics + hardware details directly from /proc.
    if is_local_ip(row.ip_host.as_deref().unwrap_or("")) {
        let h = crate::api::settings::sample_host_metrics().await;
        resp.cpu_pct = Some(h.cpu_pct);
        resp.ram_pct = Some(if h.ram_total_mb > 0 {
            ((h.ram_used_mb as f64 / h.ram_total_mb as f64) * 1000.0).round() / 10.0
        } else { 0.0 });
        resp.disk_pct = Some(h.disk_pct);
        resp.load_1 = Some(h.load_1m);
        resp.uptime_seconds = Some(h.uptime_secs as i64);
        resp.ram_total_mb = Some(h.ram_total_mb as i64);
        resp.disk_total_gb = Some(h.disk_total_gb.round() as i64);
        if resp.cpu_count.is_none() {
            resp.cpu_count = std::thread::available_parallelism().ok().map(|n| n.get() as i32);
        }
        if resp.os_version.is_none() {
            resp.os_version = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME="))
                    .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
            });
        }
    }

    Ok(Json(resp))
}

async fn create_server(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateServerRequest>,
) -> ApiResult<(StatusCode, Json<ServerResponse>)> {
    let owner_id = require_admin(&claims)?;

    let ssh_port = req.ssh_port.unwrap_or(22);
    if ssh_port < 1 || ssh_port > 65535 {
        return Err(ApiError::Validation("ssh_port must be between 1 and 65535".into()));
    }

    if req.label.is_empty() || req.label.len() > 128 {
        return Err(ApiError::Validation("label must be 1-128 characters".into()));
    }

    if req.hostname.is_empty() || req.hostname.len() > 255 {
        return Err(ApiError::Validation("hostname must be 1-255 characters".into()));
    }

    let ip_network: sqlx::types::ipnetwork::IpNetwork = req.ip_address
        .parse()
        .map_err(|_| ApiError::Validation(format!("Invalid IP address: {}", req.ip_address)))?;

    // Check for duplicate IP
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM servers WHERE ip = $1 AND deleted_at IS NULL)",
        ip_network
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::Validation(format!("Server with IP {} already exists", req.ip_address)));
    }

    let server_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO servers (id, label, hostname, ip, ssh_port, owner_id, status, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending_enrollment', NOW())",
        server_id, req.label, req.hostname, ip_network, ssh_port, owner_id
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(ServerResponse {
        id:            server_id,
        label:         req.label,
        ip:            req.ip_address,
        ssh_port,
        status:        "pending_enrollment".into(),
        agent_version: None,
        os_info:       None,
        enrolled_at:   None,
        last_seen_at:  None,
        created_at:    Utc::now(),
    })))
}

async fn update_server(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateServerRequest>,
) -> ApiResult<Json<ServerDetailResponse>> {
    require_admin(&claims)?;

    let _ = sqlx::query!(
        "SELECT id FROM servers WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "server" })?;

    if let Some(ref label) = req.label {
        if label.is_empty() || label.len() > 64 {
            return Err(ApiError::Validation("label must be 1-64 characters".into()));
        }
        sqlx::query!("UPDATE servers SET label = $1 WHERE id = $2", label, id)
            .execute(&state.db).await?;
    }

    if let Some(port) = req.ssh_port {
        if port < 1 || port > 65535 {
            return Err(ApiError::Validation("ssh_port must be between 1 and 65535".into()));
        }
        sqlx::query!("UPDATE servers SET ssh_port = $1 WHERE id = $2", port, id)
            .execute(&state.db).await?;
    }

    get_server(State(state), claims, Path(id)).await
}

async fn delete_server(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    // Check server has no active sites
    let active_sites: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM sites WHERE server_id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if active_sites > 0 {
        return Err(ApiError::Validation(format!(
            "Cannot delete server with {} active site(s). Move or delete sites first.",
            active_sites
        )));
    }

    let affected = sqlx::query!(
        "UPDATE servers SET deleted_at = NOW(), status = 'deleted' WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "server" });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn enroll_server(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(server_id): Path<Uuid>,
) -> ApiResult<Json<EnrollTokenResponse>> {
    require_admin(&claims)?;

    let _ = sqlx::query!(
        "SELECT id FROM servers WHERE id = $1 AND deleted_at IS NULL",
        server_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "server" })?;

    // Generate a single-use enrollment token (256 bits of randomness)
    let token: String = {
        use rand::distributions::Alphanumeric;
        use rand::Rng;
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64)
            .map(char::from)
            .collect()
    };

    let ttl_secs: u64 = 3600; // 1 hour

    // Store token in Valkey: orbit:enrollment:{token} -> server_id, 1h TTL, single-use
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn
            .set_ex(
                format!("orbit:enrollment:{}", token),
                server_id.to_string(),
                ttl_secs,
            )
            .await
            .unwrap_or(());
    }

    Ok(Json(EnrollTokenResponse {
        token,
        server_id,
        expires_in: ttl_secs,
    }))
}

async fn get_metrics(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(server_id): Path<Uuid>,
) -> ApiResult<Json<ServerMetricsResponse>> {
    require_admin(&claims)?;

    let _ = sqlx::query!(
        "SELECT id FROM servers WHERE id = $1 AND deleted_at IS NULL",
        server_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "server" })?;

    // ram_total_mb lives on the servers table; the rest come from server_metrics
    let server_info = sqlx::query!(
        "SELECT ram_total_mb, host(ip) as ip_text FROM servers WHERE id = $1",
        server_id
    )
    .fetch_one(&state.db)
    .await?;

    // Local (panel) host: read live metrics directly from /proc — no agent required.
    if is_local_ip(server_info.ip_text.as_deref().unwrap_or("")) {
        let h = crate::api::settings::sample_host_metrics().await;
        return Ok(Json(ServerMetricsResponse {
            server_id,
            cpu_pct:       h.cpu_pct,
            ram_used_mb:   h.ram_used_mb as i64,
            ram_total_mb:  h.ram_total_mb as i64,
            disk_used_gb:  h.disk_used_gb,
            disk_total_gb: h.disk_total_gb,
            net_in_mbps:   0.0,
            net_out_mbps:  0.0,
            load_avg_1m:   h.load_1m,
            recorded_at:   chrono::Utc::now(),
        }));
    }

    let metrics = sqlx::query!(
        "SELECT cpu_pct, ram_used_mb, disk_used_gb,
                net_in_bytes, net_out_bytes, load_1min, recorded_at
         FROM server_metrics WHERE server_id = $1
         ORDER BY recorded_at DESC LIMIT 1",
        server_id
    )
    .fetch_optional(&state.db)
    .await?;

    match metrics {
        Some(m) => Ok(Json(ServerMetricsResponse {
            server_id,
            cpu_pct:       m.cpu_pct.map(|d| d.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0),
            ram_used_mb:   m.ram_used_mb.unwrap_or(0),
            ram_total_mb:  server_info.ram_total_mb.unwrap_or(0),
            disk_used_gb:  m.disk_used_gb.map(|v| v as f64 / 1.0).unwrap_or(0.0),
            disk_total_gb: 0.0,
            net_in_mbps:   m.net_in_bytes.map(|v| v as f64 / 1_000_000.0).unwrap_or(0.0),
            net_out_mbps:  m.net_out_bytes.map(|v| v as f64 / 1_000_000.0).unwrap_or(0.0),
            load_avg_1m:   m.load_1min.map(|d| d.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0),
            recorded_at:   m.recorded_at,
        })),
        None => Ok(Json(ServerMetricsResponse {
            server_id,
            cpu_pct:       0.0,
            ram_used_mb:   0,
            ram_total_mb:  server_info.ram_total_mb.unwrap_or(0),
            disk_used_gb:  0.0,
            disk_total_gb: 0.0,
            net_in_mbps:   0.0,
            net_out_mbps:  0.0,
            load_avg_1m:   0.0,
            recorded_at:   chrono::Utc::now(),
        })),
    }
}

async fn test_connectivity(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(server_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    let server = sqlx::query!(
        "SELECT id, host(ip) as ip_text, agent_version FROM servers
         WHERE id = $1 AND deleted_at IS NULL",
        server_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "server" })?;

    let ip = server.ip_text.unwrap_or_default();

    // The local (panel) host manages itself directly — no agent/TCP hop needed.
    if is_local_ip(&ip) {
        return Ok(Json(serde_json::json!({
            "server_id": server_id,
            "ip_address": ip,
            "agent_port": 7443,
            "reachable": true,
            "agent_version": server.agent_version,
            "latency_ms": 0,
            "message": "Local panel host — managed directly (no agent required)",
        })));
    }

    // Test TCP connectivity to the orbit-agent gRPC port (7443)
    let addr = format!("{}:7443", ip);
    let reachable = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);

    Ok(Json(serde_json::json!({
        "server_id": server_id,
        "ip_address": ip,
        "agent_port": 7443,
        "reachable": reachable,
        "agent_version": server.agent_version,
        "latency_ms": null,
        "message": if reachable { "orbit-agent is reachable" } else { "Cannot reach orbit-agent on port 7443" },
    })))
}

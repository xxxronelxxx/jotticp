use axum::{
    Router,
    routing::get,
    extract::{Query, State, Path, WebSocketUpgrade, ws::{WebSocket, Message}},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use uuid::Uuid;

use crate::{AppState, ApiError};
use super::auth::{Claims, decode_jwt};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub token: String,
}

#[derive(Debug, Serialize)]
struct ServerMetricsPush {
    #[serde(rename = "type")]
    msg_type:     &'static str,
    server_id:    String,
    ts:           i64,
    cpu_pct:      f64,
    ram_pct:      f64,
    load_1:       f64,
    net_in_kbps:  f64,
    net_out_kbps: f64,
}

#[derive(Debug, Serialize)]
struct MetricsPush {
    #[serde(rename = "type")]
    msg_type:        &'static str,
    sites:           Vec<SiteMetric>,
    server_count:    i64,
    /// Total active (non-deleted) sites on the server
    active_sites:    i64,
    /// Number of SSL certs expiring in <30 days (warning level)
    ssl_warn_count:  i64,
    /// Number of SSL certs expiring in <7 days (critical level)
    ssl_crit_count:  i64,
    /// Backup jobs currently in progress
    backup_running:  i64,
    /// Last 5 audit log entries for the dashboard feed
    recent_events:   Vec<AuditEvent>,
    timestamp:       i64,
}

#[derive(Debug, Serialize)]
struct SiteMetric {
    id:       String,
    domain:   String,
    cpu_pct:  f64,
    mem_mb:   i64,
    disk_gb:  f64,
}

#[derive(Debug, Serialize)]
struct AuditEvent {
    action:     String,
    actor:      String,
    target:     Option<String>,
    occurred_at: i64,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/ws", get(ws_handler))
        .route("/ws/metrics/{server_id}", get(ws_metrics_handler))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    // Authenticate via token query param before upgrading
    let claims_result = authenticate_ws_token(&state, &q.token).await;

    match claims_result {
        Ok(claims) => ws.on_upgrade(move |socket| handle_ws(socket, state, claims)),
        Err(_) => {
            // Upgrade and immediately close with policy violation (4001 = unauthorized)
            ws.on_upgrade(|mut socket| async move {
                let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 4001,
                    reason: "Unauthorized".into(),
                }))).await;
            })
        }
    }
}

async fn ws_metrics_handler(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<Uuid>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    let claims_result = authenticate_ws_token(&state, &q.token).await;
    match claims_result {
        Ok(_) => ws.on_upgrade(move |socket| handle_server_metrics_ws(socket, state, server_id)),
        Err(_) => ws.on_upgrade(|mut socket| async move {
            let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 4001,
                reason: "Unauthorized".into(),
            }))).await;
        }),
    }
}

async fn handle_server_metrics_ws(socket: WebSocket, state: Arc<AppState>, server_id: Uuid) {
    let (mut sender, mut receiver) = socket.split();

    let ram_total: i64 = sqlx::query_scalar!(
        "SELECT ram_total_mb FROM servers WHERE id = $1",
        server_id
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .flatten()
    .unwrap_or(1024);

    let mut tick = interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let metrics_opt = sqlx::query!(
                    "SELECT cpu_pct, ram_used_mb, load_1min, net_in_bytes, net_out_bytes
                     FROM server_metrics WHERE server_id = $1
                     ORDER BY recorded_at DESC LIMIT 1",
                    server_id
                )
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();

                let (cpu_pct, ram_pct, load_1, net_in_kbps, net_out_kbps) = match metrics_opt {
                    Some(m) => {
                        let cpu = m.cpu_pct.and_then(|d| d.to_string().parse::<f64>().ok()).unwrap_or(0.0);
                        let ram_mb = m.ram_used_mb.unwrap_or(0);
                        let ram_p = if ram_total > 0 { (ram_mb as f64 / ram_total as f64) * 100.0 } else { 0.0 };
                        let load = m.load_1min.and_then(|d| d.to_string().parse::<f64>().ok()).unwrap_or(0.0);
                        let net_in = m.net_in_bytes.map(|v| v as f64 / 1000.0).unwrap_or(0.0);
                        let net_out = m.net_out_bytes.map(|v| v as f64 / 1000.0).unwrap_or(0.0);
                        (cpu, ram_p, load, net_in, net_out)
                    }
                    None => (0.0, 0.0, 0.0, 0.0, 0.0),
                };

                let payload = ServerMetricsPush {
                    msg_type:     "metrics",
                    server_id:    server_id.to_string(),
                    ts:           chrono::Utc::now().timestamp_millis(),
                    cpu_pct,
                    ram_pct,
                    load_1,
                    net_in_kbps,
                    net_out_kbps,
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn authenticate_ws_token(state: &AppState, token: &str) -> Result<Claims, ApiError> {
    let claims = decode_jwt(token, &state.jwt_decoding_key)
        .map_err(|_| ApiError::Unauthorized)?;

    if !claims.totp_verified {
        return Err(ApiError::Unauthorized);
    }

    // JTI blocklist check
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let blocked: Option<String> = conn
            .get(format!("orbit:blocklist:{}", claims.jti))
            .await
            .unwrap_or(None);
        if blocked.is_some() {
            return Err(ApiError::Unauthorized);
        }
    }

    Ok(claims)
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>, claims: Claims) {
    let (mut sender, mut receiver) = socket.split();

    let user_id: uuid::Uuid = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => return,
    };
    let is_admin = claims.role == "admin";

    tracing::info!(user_id = %user_id, "WebSocket connected");

    let mut tick = interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                match build_metrics_push(&state, user_id, is_admin).await {
                    Ok(payload) => {
                        let json = match serde_json::to_string(&payload) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::error!("WS serialize error: {}", e);
                                break;
                            }
                        };
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            // Client disconnected
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("WS metrics error: {}", e);
                        // Continue — don't break on DB errors
                    }
                }
            }

            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!(user_id = %user_id, "WebSocket disconnected");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {} // Ignore other message types
                    Some(Err(e)) => {
                        tracing::warn!(user_id = %user_id, "WebSocket error: {}", e);
                        break;
                    }
                }
            }
        }
    }

    let _ = sender.close().await;
}

async fn build_metrics_push(
    state: &AppState,
    user_id: uuid::Uuid,
    is_admin: bool,
) -> anyhow::Result<MetricsPush> {
    // ── Per-site metrics ──────────────────────────────────────────────────────
    let site_rows = sqlx::query!(
        r#"SELECT s.id, s.domain, s.unix_user,
                  sm.cpu_pct, sm.ram_used_mb, sm.disk_used_gb
           FROM sites s
           LEFT JOIN LATERAL (
               SELECT cpu_pct, ram_used_mb, disk_used_gb
               FROM server_metrics
               WHERE server_id = s.server_id
               ORDER BY recorded_at DESC
               LIMIT 1
           ) sm ON true
           WHERE s.deleted_at IS NULL
             AND ($1::boolean OR s.owner_id = $2)
           ORDER BY s.domain
           LIMIT 50"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    let sites: Vec<SiteMetric> = site_rows.into_iter().map(|r| {
        // Prefer cgroup-based memory when available (more accurate than DB snapshot)
        let mem_from_cgroup = read_cgroup_memory_mb(&r.unix_user);
        SiteMetric {
            id:      r.id.to_string(),
            domain:  r.domain,
            cpu_pct: r.cpu_pct
                .and_then(|d| d.to_string().parse::<f64>().ok())
                .unwrap_or(0.0),
            mem_mb:  mem_from_cgroup.unwrap_or(r.ram_used_mb.unwrap_or(0)),
            disk_gb: r.disk_used_gb.map(|v| v as f64).unwrap_or(0.0),
        }
    }).collect();

    // ── Server count ──────────────────────────────────────────────────────────
    let server_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM servers WHERE deleted_at IS NULL AND status != 'deleted'"
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    // ── Active sites count ────────────────────────────────────────────────────
    let active_sites: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM sites
           WHERE deleted_at IS NULL AND status = 'active'
             AND ($1::boolean OR owner_id = $2)"#,
        is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    // ── SSL expiry warnings ───────────────────────────────────────────────────
    let ssl_warn_count: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM ssl_certs
           WHERE expires_at < NOW() + INTERVAL '30 days'
             AND expires_at > NOW() + INTERVAL '7 days'
             AND status = 'active'"#
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let ssl_crit_count: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM ssl_certs
           WHERE expires_at < NOW() + INTERVAL '7 days'
             AND status = 'active'"#
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    // ── Backup jobs in progress ───────────────────────────────────────────────
    let backup_running: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM backups WHERE status = 'running'"
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    // ── Recent audit events (last 5 entries) ──────────────────────────────────
    let audit_rows = sqlx::query!(
        r#"SELECT a.action,
                  COALESCE(u.email::text, 'system') AS "actor_email!: String",
                  a.target_id, a.created_at
           FROM audit_log a
           LEFT JOIN users u ON u.id = a.user_id
           ORDER BY a.created_at DESC
           LIMIT 5"#
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let recent_events: Vec<AuditEvent> = audit_rows.into_iter().map(|r| AuditEvent {
        action:      r.action,
        actor:       r.actor_email,
        target:      r.target_id.map(|u| u.to_string()),
        occurred_at: r.created_at.timestamp(),
    }).collect();

    Ok(MetricsPush {
        msg_type:       "metrics",
        sites,
        server_count,
        active_sites,
        ssl_warn_count,
        ssl_crit_count,
        backup_running,
        recent_events,
        timestamp:      chrono::Utc::now().timestamp(),
    })
}

fn read_cgroup_memory_mb(unix_user: &str) -> Option<i64> {
    let path = format!("/sys/fs/cgroup/website-{}.slice/memory.current", unix_user);
    let bytes: u64 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
    Some((bytes / 1024 / 1024) as i64)
}

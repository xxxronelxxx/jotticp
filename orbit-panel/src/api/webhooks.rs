use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Webhook {
    pub id:         Uuid,
    pub name:       String,
    pub url:        String,
    pub events:     Vec<String>,
    pub enabled:    bool,
    pub secret:     Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name:    String,
    pub url:     String,
    pub events:  Vec<String>,
    pub secret:  Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub name:    Option<String>,
    pub url:     Option<String>,
    pub events:  Option<Vec<String>>,
    pub secret:  Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub success:      bool,
    pub http_status:  Option<u16>,
    pub error:        Option<String>,
}

// Allowed event types
const VALID_EVENTS: &[&str] = &[
    "site.created", "site.deleted", "site.suspended", "site.unsuspended",
    "ssl.issued", "ssl.expiring", "ssl.expired",
    "backup.completed", "backup.failed",
    "user.created", "user.deleted",
    "server.offline", "server.recovered",
    "deploy.completed", "deploy.failed",
    "malware.detected",
];

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/webhooks",          get(list_webhooks).post(create_webhook))
        .route("/api/v1/webhooks/{id}",     get(get_webhook).put(update_webhook).delete(delete_webhook))
        .route("/api/v1/webhooks/{id}/test", post(test_webhook))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

fn validate_url(url: &str) -> ApiResult<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(ApiError::Validation("Webhook URL must start with http:// or https://".into()));
    }
    if url.len() > 500 {
        return Err(ApiError::Validation("Webhook URL too long".into()));
    }
    // Reject internal addresses to prevent SSRF
    let blocked = ["127.", "10.", "192.168.", "169.254.", "::1", "localhost"];
    let lower = url.to_lowercase();
    if blocked.iter().any(|b| lower.contains(b)) {
        return Err(ApiError::Validation("Webhook URL cannot point to internal/private addresses".into()));
    }
    Ok(())
}

fn validate_events(events: &[String]) -> ApiResult<()> {
    if events.is_empty() {
        return Err(ApiError::Validation("At least one event type required".into()));
    }
    if events.len() > 20 {
        return Err(ApiError::Validation("Maximum 20 event types per webhook".into()));
    }
    for ev in events {
        if !VALID_EVENTS.contains(&ev.as_str()) {
            return Err(ApiError::Validation(
                format!("Unknown event type '{}'. Valid events: {}", ev, VALID_EVENTS.join(", "))
            ));
        }
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/webhooks
async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<serde_json::Value>>> {
    require_admin(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT id, name, url, events, enabled, created_at
           FROM webhooks ORDER BY created_at"#
    )
    .fetch_all(&state.db)
    .await?;

    let list: Vec<serde_json::Value> = rows.into_iter().map(|r| json!({
        "id":         r.id,
        "name":       r.name,
        "url":        r.url,
        "events":     r.events,
        "enabled":    r.enabled,
        "created_at": r.created_at.to_rfc3339(),
        // Never expose the secret in list view
    })).collect();

    Ok(Json(list))
}

/// POST /api/v1/webhooks
async fn create_webhook(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateWebhookRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    require_admin(&claims)?;

    if req.name.trim().is_empty() || req.name.len() > 128 {
        return Err(ApiError::Validation("name must be 1-128 characters".into()));
    }
    validate_url(&req.url)?;
    validate_events(&req.events)?;

    let row = sqlx::query!(
        r#"INSERT INTO webhooks (name, url, events, secret, enabled)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, name, url, events, enabled, created_at"#,
        req.name.trim(),
        req.url,
        &req.events,
        req.secret,
        req.enabled.unwrap_or(true)
    )
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({
        "id":         row.id,
        "name":       row.name,
        "url":        row.url,
        "events":     row.events,
        "enabled":    row.enabled,
        "created_at": row.created_at.to_rfc3339(),
    }))))
}

/// GET /api/v1/webhooks/{id}
async fn get_webhook(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    let row = sqlx::query!(
        "SELECT id, name, url, events, enabled, created_at FROM webhooks WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "webhook" })?;

    Ok(Json(json!({
        "id":         row.id,
        "name":       row.name,
        "url":        row.url,
        "events":     row.events,
        "enabled":    row.enabled,
        "created_at": row.created_at.to_rfc3339(),
    })))
}

/// PUT /api/v1/webhooks/{id}
async fn update_webhook(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateWebhookRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    if let Some(ref url) = req.url { validate_url(url)?; }
    if let Some(ref evs) = req.events { validate_events(evs)?; }

    let row = sqlx::query!(
        r#"UPDATE webhooks
           SET name    = COALESCE($1, name),
               url     = COALESCE($2, url),
               events  = COALESCE($3, events),
               secret  = COALESCE($4, secret),
               enabled = COALESCE($5, enabled)
           WHERE id = $6
           RETURNING id, name, url, events, enabled, created_at"#,
        req.name.as_deref(),
        req.url.as_deref(),
        req.events.as_deref(),
        req.secret.as_deref(),
        req.enabled,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "webhook" })?;

    Ok(Json(json!({
        "id":         row.id,
        "name":       row.name,
        "url":        row.url,
        "events":     row.events,
        "enabled":    row.enabled,
        "created_at": row.created_at.to_rfc3339(),
    })))
}

/// DELETE /api/v1/webhooks/{id}
async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    let deleted = sqlx::query_scalar!(
        "DELETE FROM webhooks WHERE id = $1 RETURNING id",
        id
    )
    .fetch_optional(&state.db)
    .await?;

    if deleted.is_none() {
        return Err(ApiError::NotFound { resource: "webhook" });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/webhooks/{id}/test
/// Send a test payload to verify the webhook endpoint is reachable.
async fn test_webhook(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TestResult>> {
    require_admin(&claims)?;

    let row = sqlx::query!(
        "SELECT url, secret FROM webhooks WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "webhook" })?;

    let payload = json!({
        "event":    "test.ping",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "data":     { "message": "OrbitCP webhook test" },
    });

    let mut req_builder = state.http
        .post(&row.url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "OrbitCP-Webhook/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);

    // Add HMAC-SHA256 signature if secret is set
    if let Some(secret) = row.secret {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let body = serde_json::to_string(&payload).unwrap_or_default();
        if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
            mac.update(body.as_bytes());
            let sig = hex::encode(mac.finalize().into_bytes());
            req_builder = req_builder.header("X-OrbitCP-Signature", format!("sha256={}", sig));
        }
    }

    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            Ok(Json(TestResult {
                success:     status < 400,
                http_status: Some(status),
                error:       None,
            }))
        }
        Err(e) => Ok(Json(TestResult {
            success:     false,
            http_status: None,
            error:       Some(e.to_string()),
        })),
    }
}

// ── Public dispatch function (called by audit/event system) ───────────────────

/// Fire all enabled webhooks matching the given event type.
/// Called from middleware/audit.rs on POST/PUT/DELETE events.
pub async fn dispatch_event(state: &AppState, event: &str, data: serde_json::Value) {
    let rows = match sqlx::query!(
        "SELECT url, secret FROM webhooks WHERE enabled = true AND $1 = ANY(events)",
        event
    )
    .fetch_all(&state.db)
    .await {
        Ok(r) => r,
        Err(_) => return,
    };

    let payload = json!({
        "event":     event,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "data":      data,
    });

    for row in rows {
        let client = state.http.clone();
        let url    = row.url.clone();
        let secret = row.secret.clone();
        let p      = payload.clone();

        tokio::spawn(async move {
            let mut rb = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("User-Agent", "OrbitCP-Webhook/1.0")
                .timeout(std::time::Duration::from_secs(10))
                .json(&p);

            if let Some(sec) = secret {
                use hmac::{Hmac, Mac};
                use sha2::Sha256;
                type HmacSha256 = Hmac<Sha256>;
                let body = serde_json::to_string(&p).unwrap_or_default();
                if let Ok(mut mac) = HmacSha256::new_from_slice(sec.as_bytes()) {
                    mac.update(body.as_bytes());
                    let sig = hex::encode(mac.finalize().into_bytes());
                    rb = rb.header("X-OrbitCP-Signature", format!("sha256={}", sig));
                }
            }

            if let Err(e) = rb.send().await {
                tracing::warn!(url = %url, error = %e, "Webhook delivery failed");
            }
        });
    }
}

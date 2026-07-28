use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CertResponse {
    pub id:          Uuid,
    pub site_id:     Uuid,
    pub domain:      String,
    pub status:      String,
    pub cert_path:   Option<String>,
    pub key_path:    Option<String>,
    pub issued_at:   Option<DateTime<Utc>>,
    pub expires_at:  Option<DateTime<Utc>>,
    pub auto_renew:  bool,
    pub issuer:      Option<String>,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct IssueWildcardRequest {
    /// DNS provider for DNS-01 challenge (e.g. "powerdns" | "cloudflare")
    pub dns_provider: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/ssl/certs",                        get(list_certs))
        .route("/api/v1/ssl/certs/{site_id}/issue",         post(issue_cert))
        .route("/api/v1/ssl/certs/{site_id}/issue-wildcard", post(issue_wildcard))
        .route("/api/v1/ssl/certs/{site_id}/upload",        post(upload_cert))
        .route("/api/v1/ssl/certs/{site_id}",               delete(delete_cert))
        .route("/api/v1/ssl/certs/{site_id}/status",        get(cert_status))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_certs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<CertResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT sc.id, sc.site_id AS "site_id!", sc.domain, sc.status::text AS "status!", sc.cert_path,
                  sc.key_path, sc.issued_at, sc.expires_at, sc.auto_renew, sc.issuer, sc.created_at
           FROM ssl_certs sc
           JOIN sites s ON s.id = sc.site_id
           WHERE s.deleted_at IS NULL
             AND ($1::boolean OR s.owner_id = $2)
           ORDER BY sc.created_at DESC"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| CertResponse {
        id:         r.id,
        site_id:    r.site_id,
        domain:     r.domain,
        status:     r.status,
        cert_path:  r.cert_path,
        key_path:   r.key_path,
        issued_at:  r.issued_at,
        expires_at: r.expires_at,
        auto_renew: r.auto_renew,
        issuer:     r.issuer,
        created_at: r.created_at,
    }).collect()))
}

async fn cert_status(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<CertResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT sc.id, sc.site_id AS "site_id!", sc.domain, sc.status::text AS "status!", sc.cert_path,
                  sc.key_path, sc.issued_at, sc.expires_at, sc.auto_renew, sc.issuer, sc.created_at
           FROM ssl_certs sc
           JOIN sites s ON s.id = sc.site_id
           WHERE sc.site_id = $1
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)
           ORDER BY sc.created_at DESC
           LIMIT 1"#,
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "ssl_cert" })?;

    Ok(Json(CertResponse {
        id:         row.id,
        site_id:    row.site_id,
        domain:     row.domain,
        status:     row.status,
        cert_path:  row.cert_path,
        key_path:   row.key_path,
        issued_at:  row.issued_at,
        expires_at: row.expires_at,
        auto_renew: row.auto_renew,
        issuer:     row.issuer,
        created_at: row.created_at,
    }))
}

async fn issue_cert(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT id, domain FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Upsert cert record in pending state
    let cert_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO ssl_certs (id, site_id, domain, status, auto_renew, issuer, created_at)
         VALUES ($1, $2, $3, 'pending', true, 'letsencrypt', NOW())
         ON CONFLICT (site_id) DO UPDATE SET status = 'pending', issuer = 'letsencrypt'",
        cert_id, site_id, site.domain
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "issue_ssl",
            "cert_id": cert_id,
            "site_id": site_id,
            "domain": site.domain,
            "challenge": "http-01",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({
        "status": "pending",
        "cert_id": cert_id,
        "domain": site.domain,
        "challenge": "http-01",
        "message": "Let's Encrypt HTTP-01 challenge queued. Certificate will be ready in ~30 seconds.",
    }))))
}

async fn issue_wildcard(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<IssueWildcardRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // Wildcard SSL requires Pro
    if !state.config.is_pro() {
        return Err(ApiError::FeatureGated { feature: "wildcard_ssl" });
    }

    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT id, domain FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let wildcard_domain = format!("*.{}", site.domain);
    let cert_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO ssl_certs (id, site_id, domain, status, auto_renew, issuer, created_at)
         VALUES ($1, $2, $3, 'pending', true, 'letsencrypt', NOW())
         ON CONFLICT (site_id) DO UPDATE SET status = 'pending', domain = $3, issuer = 'letsencrypt'",
        cert_id, site_id, wildcard_domain
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "issue_ssl",
            "cert_id": cert_id,
            "site_id": site_id,
            "domain": wildcard_domain,
            "challenge": "dns-01",
            "dns_provider": req.dns_provider.unwrap_or_else(|| "powerdns".to_string()),
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({
        "status": "pending",
        "cert_id": cert_id,
        "domain": wildcard_domain,
        "challenge": "dns-01",
        "message": "Wildcard DNS-01 challenge queued. A TXT record will be created in your DNS zone.",
    }))))
}

async fn upload_cert(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // Custom cert upload requires Pro
    if !state.config.is_pro() {
        return Err(ApiError::FeatureGated { feature: "custom_ssl_upload" });
    }

    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT id, domain FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let mut cert_pem: Option<Vec<u8>> = None;
    let mut key_pem: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await
        .map_err(|e| ApiError::Validation(format!("Multipart error: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        let data = field.bytes().await
            .map_err(|e| ApiError::Validation(format!("Field read error: {}", e)))?;

        if data.len() > 1024 * 1024 {
            return Err(ApiError::Validation("Certificate or key file too large (max 1MB)".into()));
        }

        match name.as_str() {
            "cert.pem" | "cert" => cert_pem = Some(data.to_vec()),
            "key.pem" | "key"   => key_pem  = Some(data.to_vec()),
            _ => {}
        }
    }

    let cert_pem = cert_pem.ok_or_else(|| ApiError::Validation("Missing cert.pem field".into()))?;
    let key_pem  = key_pem.ok_or_else(|| ApiError::Validation("Missing key.pem field".into()))?;

    // Basic PEM validation
    let cert_str = std::str::from_utf8(&cert_pem)
        .map_err(|_| ApiError::Validation("cert.pem is not valid UTF-8".into()))?;
    let key_str = std::str::from_utf8(&key_pem)
        .map_err(|_| ApiError::Validation("key.pem is not valid UTF-8".into()))?;

    if !cert_str.contains("-----BEGIN CERTIFICATE-----") {
        return Err(ApiError::Validation("cert.pem does not appear to be a PEM certificate".into()));
    }
    if !key_str.contains("-----BEGIN") {
        return Err(ApiError::Validation("key.pem does not appear to be a PEM private key".into()));
    }

    // Write cert files to disk under /etc/jottiecp/ssl/{site_id}/
    let cert_dir = format!("/etc/jottiecp/ssl/{}", site_id);
    let cert_path = format!("{}/cert.pem", cert_dir);
    let key_path  = format!("{}/key.pem", cert_dir);

    tokio::fs::create_dir_all(&cert_dir).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create cert dir: {}", e)))?;
    tokio::fs::write(&cert_path, &cert_pem).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot write cert: {}", e)))?;
    tokio::fs::write(&key_path, &key_pem).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot write key: {}", e)))?;

    // Restrict key file permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chmod key: {}", e)))?;
    }

    let cert_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO ssl_certs (id, site_id, domain, status, cert_path, key_path, auto_renew, issuer, created_at)
         VALUES ($1, $2, $3, 'active', $4, $5, false, 'custom', NOW())
         ON CONFLICT (site_id) DO UPDATE SET
           status = 'active', cert_path = $4, key_path = $5, auto_renew = false, issuer = 'custom'",
        cert_id, site_id, site.domain, cert_path, key_path
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "activate_custom_ssl",
            "cert_id": cert_id,
            "site_id": site_id,
            "domain": site.domain,
            "cert_path": cert_path,
            "key_path": key_path,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "cert_id": cert_id,
        "domain": site.domain,
        "issuer": "custom",
        "status": "active",
    }))))
}

async fn delete_cert(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let cert = sqlx::query!(
        r#"SELECT sc.id, sc.cert_path, sc.key_path, sc.domain FROM ssl_certs sc
           JOIN sites s ON s.id = sc.site_id
           WHERE sc.site_id = $1 AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)
           LIMIT 1"#,
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "ssl_cert" })?;

    sqlx::query!("DELETE FROM ssl_certs WHERE site_id = $1", site_id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "revoke_ssl",
            "cert_id": cert.id,
            "site_id": site_id,
            "domain": cert.domain,
            "cert_path": cert.cert_path,
            "key_path": cert.key_path,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

const CF_API: &str = "https://api.cloudflare.com/client/v4";

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CloudflareToken {
    pub id:         Uuid,
    pub label:      String,
    pub zone_id:    String,
    pub domain:     String,
    pub proxy:      bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AddTokenRequest {
    pub label:   String,
    pub api_token: String,
    pub zone_id: String,
    pub domain:  String,
    /// Proxy all A/AAAA records through Cloudflare (orange-cloud).
    pub proxy:   Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    /// If true, enable orange-cloud proxy on synced records.
    pub proxy: Option<bool>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/cloudflare/tokens",               get(list_tokens).post(add_token))
        .route("/api/v1/cloudflare/tokens/{id}",           delete(delete_token))
        .route("/api/v1/cloudflare/tokens/{id}/sync",      post(sync_zone))
        .route("/api/v1/cloudflare/tokens/{id}/proxy",     put(set_proxy))
        .route("/api/v1/cloudflare/zones",                get(list_cf_zones))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        Err(ApiError::Forbidden("Admin access required".into()))
    } else {
        Ok(())
    }
}

/// Make a Cloudflare API request with the stored API token.
async fn cf_get(client: &reqwest::Client, token: &str, path: &str) -> ApiResult<Value> {
    let resp = client
        .get(format!("{}{}", CF_API, path))
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| ApiError::ExternalService(format!("Cloudflare request: {}", e)))?;

    let body: Value = resp.json().await
        .map_err(|e| ApiError::ExternalService(format!("Cloudflare JSON: {}", e)))?;

    if !body.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let errors = body.get("errors").cloned().unwrap_or(Value::Array(vec![]));
        return Err(ApiError::ExternalService(format!("Cloudflare error: {}", errors)));
    }
    Ok(body)
}

async fn cf_post(client: &reqwest::Client, token: &str, path: &str, body: &Value) -> ApiResult<Value> {
    let resp = client
        .post(format!("{}{}", CF_API, path))
        .bearer_auth(token)
        .json(body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| ApiError::ExternalService(format!("Cloudflare request: {}", e)))?;

    let body: Value = resp.json().await
        .map_err(|e| ApiError::ExternalService(format!("Cloudflare JSON: {}", e)))?;

    if !body.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let errors = body.get("errors").cloned().unwrap_or_default();
        return Err(ApiError::ExternalService(format!("Cloudflare error: {}", errors)));
    }
    Ok(body)
}

async fn cf_delete(client: &reqwest::Client, token: &str, path: &str) -> ApiResult<()> {
    let resp = client
        .delete(format!("{}{}", CF_API, path))
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| ApiError::ExternalService(format!("Cloudflare request: {}", e)))?;

    let body: Value = resp.json().await.unwrap_or(json!({"success": true}));
    if !body.get("success").and_then(|v| v.as_bool()).unwrap_or(true) {
        let errors = body.get("errors").cloned().unwrap_or_default();
        return Err(ApiError::ExternalService(format!("Cloudflare error: {}", errors)));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/cloudflare/tokens
async fn list_tokens(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<Value>>> {
    require_admin(&claims)?;

    let rows = sqlx::query!(
        "SELECT id, label, zone_id, domain, proxy, created_at FROM cloudflare_tokens ORDER BY created_at"
    )
    .fetch_all(&state.db)
    .await?;

    let list: Vec<Value> = rows.into_iter().map(|r| json!({
        "id":         r.id,
        "label":      r.label,
        "zone_id":    r.zone_id,
        "domain":     r.domain,
        "proxy":      r.proxy,
        "created_at": r.created_at.to_rfc3339(),
        // api_token intentionally omitted from response
    })).collect();

    Ok(Json(list))
}

/// POST /api/v1/cloudflare/tokens
async fn add_token(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddTokenRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_admin(&claims)?;

    if req.label.trim().is_empty() || req.label.len() > 128 {
        return Err(ApiError::Validation("label must be 1-128 characters".into()));
    }
    if req.api_token.trim().is_empty() {
        return Err(ApiError::Validation("api_token is required".into()));
    }
    if req.zone_id.trim().is_empty() || !req.zone_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::Validation("zone_id must be a 32-char hex string".into()));
    }

    // Validate the token actually works by calling /user/tokens/verify
    let verify = cf_get(&state.http, &req.api_token, "/user/tokens/verify").await;
    if let Err(e) = verify {
        return Err(ApiError::Validation(format!("Cloudflare token verification failed: {}", e)));
    }

    // Encrypt the API token at rest (XOR with ORBIT_BACKUP_KEY / fallback to zeros)
    let api_token_enc = encrypt_token(&req.api_token, &state.config);

    let row = sqlx::query!(
        r#"INSERT INTO cloudflare_tokens (label, api_token_enc, zone_id, domain, proxy)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, label, zone_id, domain, proxy, created_at"#,
        req.label.trim(),
        api_token_enc,
        req.zone_id.trim(),
        req.domain.to_lowercase().trim().to_string(),
        req.proxy.unwrap_or(false)
    )
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({
        "id":         row.id,
        "label":      row.label,
        "zone_id":    row.zone_id,
        "domain":     row.domain,
        "proxy":      row.proxy,
        "created_at": row.created_at.to_rfc3339(),
    }))))
}

/// DELETE /api/v1/cloudflare/tokens/{id}
async fn delete_token(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    let deleted = sqlx::query_scalar!(
        "DELETE FROM cloudflare_tokens WHERE id = $1 RETURNING id", id
    )
    .fetch_optional(&state.db)
    .await?;

    if deleted.is_none() {
        return Err(ApiError::NotFound { resource: "Cloudflare token" });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/cloudflare/tokens/{id}/sync
/// Sync all DNS records from JottiCP → Cloudflare for this zone.
async fn sync_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<Option<SyncRequest>>,
) -> ApiResult<Json<Value>> {
    require_admin(&claims)?;

    let cf_row = sqlx::query!(
        "SELECT api_token_enc, zone_id, domain, proxy FROM cloudflare_tokens WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "Cloudflare token" })?;

    let api_token = decrypt_token(&cf_row.api_token_enc, &state.config);
    let proxy = req.as_ref().and_then(|r| r.proxy).unwrap_or(cf_row.proxy);

    // Fetch all DNS records for this domain from JottiCP DB
    let records = sqlx::query!(
        r#"SELECT dr.type AS record_type, dr.name, dr.content, dr.ttl, dr.priority
           FROM dns_records dr
           JOIN dns_zones dz ON dz.id = dr.zone_id
           WHERE dz.domain = $1 AND dr.deleted_at IS NULL"#,
        cf_row.domain
    )
    .fetch_all(&state.db)
    .await?;

    // Get existing CF records to avoid duplicates
    let existing = cf_get(&state.http, &api_token,
        &format!("/zones/{}/dns_records?per_page=100", cf_row.zone_id))
        .await?;

    let existing_ids: Vec<(String, String, String)> = existing
        .get("result")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|r| {
            Some((
                r.get("type")?.as_str()?.to_string(),
                r.get("name")?.as_str()?.to_string(),
                r.get("id")?.as_str()?.to_string(),
            ))
        }).collect())
        .unwrap_or_default();

    let mut synced = 0usize;
    let mut errors = 0usize;

    for rec in &records {
        let rtype = &rec.record_type;
        // Skip SOA, NS at root — CF manages those
        if rtype == "SOA" || rtype == "NS" { continue; }

        // Check if record exists in CF already
        let cf_name = if rec.name == "@" || rec.name == cf_row.domain {
            cf_row.domain.clone()
        } else {
            format!("{}.{}", rec.name, cf_row.domain)
        };

        let existing_id = existing_ids.iter()
            .find(|(t, n, _)| t == rtype && n == &cf_name)
            .map(|(_, _, id)| id.clone());

        let mut payload = json!({
            "type":    rtype,
            "name":    cf_name,
            "content": rec.content,
            "ttl":     if rec.ttl == 1 { 1 } else { rec.ttl }, // 1 = auto
        });

        if let Some(prio) = rec.priority {
            payload["priority"] = json!(prio);
        }

        // Enable proxy only for A and AAAA records
        if proxy && (rtype == "A" || rtype == "AAAA") {
            payload["proxied"] = json!(true);
        }

        let result = if let Some(eid) = existing_id {
            // Update existing record
            cf_post(&state.http, &api_token,
                &format!("/zones/{}/dns_records/{}", cf_row.zone_id, eid),
                &payload).await
        } else {
            // Create new record
            cf_post(&state.http, &api_token,
                &format!("/zones/{}/dns_records", cf_row.zone_id),
                &payload).await
        };

        if result.is_ok() { synced += 1; } else { errors += 1; }
    }

    Ok(Json(json!({
        "synced": synced,
        "errors": errors,
        "message": format!("Synced {} records to Cloudflare ({} errors)", synced, errors),
    })))
}

/// PUT /api/v1/cloudflare/tokens/{id}/proxy — toggle proxy (orange-cloud) on all records
async fn set_proxy(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    require_admin(&claims)?;

    let proxy = body.get("proxy").and_then(|v| v.as_bool())
        .ok_or_else(|| ApiError::Validation("proxy (bool) required".into()))?;

    sqlx::query!(
        "UPDATE cloudflare_tokens SET proxy = $1 WHERE id = $2", proxy, id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "proxy": proxy, "message": "Proxy setting updated. Run /sync to apply." })))
}

/// GET /api/v1/cloudflare/zones — list zones visible to any stored token
async fn list_cf_zones(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<Value>>> {
    require_admin(&claims)?;

    let tokens = sqlx::query!("SELECT api_token_enc FROM cloudflare_tokens")
        .fetch_all(&state.db)
        .await?;

    let mut all_zones = Vec::new();
    for t in &tokens {
        let api_token = decrypt_token(&t.api_token_enc, &state.config);
        if let Ok(resp) = cf_get(&state.http, &api_token, "/zones?per_page=50").await {
            if let Some(arr) = resp.get("result").and_then(|r| r.as_array()) {
                for z in arr {
                    all_zones.push(json!({
                        "id":     z.get("id"),
                        "name":   z.get("name"),
                        "status": z.get("status"),
                    }));
                }
            }
        }
    }

    Ok(Json(all_zones))
}

// ── Encryption helpers ────────────────────────────────────────────────────────

fn encrypt_token(token: &str, config: &crate::config::Config) -> String {
    // XOR with a key derived from ORBIT_INTERNAL_TOKEN — simple obfuscation.
    // In production, replace with AES-256-GCM via the `aes-gcm` crate.
    let fallback = "jotticp_default_key_change_me".to_string();
    let key = config.internal_token.as_deref().unwrap_or(&fallback).as_bytes();
    let enc: Vec<u8> = token.bytes()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &enc)
}

fn decrypt_token(enc: &str, config: &crate::config::Config) -> String {
    let fallback = "jotticp_default_key_change_me".to_string();
    let key = config.internal_token.as_deref().unwrap_or(&fallback).as_bytes();
    if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, enc) {
        let dec: Vec<u8> = bytes.iter()
            .enumerate()
            .map(|(i, &b)| b ^ key[i % key.len()])
            .collect();
        String::from_utf8(dec).unwrap_or_default()
    } else {
        String::new()
    }
}

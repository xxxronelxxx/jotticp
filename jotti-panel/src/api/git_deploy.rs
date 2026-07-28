use axum::{
    Router,
    routing::{get, post},
    extract::{Path, State},
    Json,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use crate::services::git_deploy as svc;
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct GitDeployStatus {
    pub enabled:     bool,
    pub repo_path:   Option<String>,
    pub remote_url:  Option<String>,
    pub last_deploy: Option<DeployInfo>,
}

#[derive(Debug, Serialize)]
pub struct DeployInfo {
    pub hash:        String,
    pub message:     Option<String>,
    pub deployed_at: DateTime<Utc>,
    pub deployed_by: String,
}

#[derive(Debug, Deserialize)]
pub struct RecordDeployRequest {
    pub hash:        String,
    pub commit_msg:  Option<String>,
    pub deployed_at: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Status / management (require admin JWT)
        .route(
            "/api/v1/sites/{id}/git-deploy/status",
            get(get_status),
        )
        .route(
            "/api/v1/sites/{id}/git-deploy/enable",
            post(enable_deploy),
        )
        .route(
            "/api/v1/sites/{id}/git-deploy/disable",
            post(disable_deploy),
        )
        .route(
            "/api/v1/sites/{id}/git-deploy/history",
            get(get_history),
        )
        .route(
            "/api/v1/sites/{id}/git-deploy/trigger",
            post(trigger_deploy),
        )
        // Internal endpoint — called by the post-receive git hook, not by the UI
        .route(
            "/api/v1/sites/{id}/git-deploy/record",
            post(record_deploy_internal),
        )
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/sites/{id}/git-deploy/status
async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    claims: Claims,
) -> ApiResult<Json<GitDeployStatus>> {
    require_admin(&claims)?;

    let site = sqlx::query!(
        r#"SELECT git_deploy_enabled, git_repo_path
           FROM sites
           WHERE id = $1 AND deleted_at IS NULL"#,
        site_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let last_deploy = sqlx::query!(
        r#"SELECT commit_hash, commit_msg, deployed_at, deployed_by
           FROM git_deploys
           WHERE site_id = $1
           ORDER BY deployed_at DESC
           LIMIT 1"#,
        site_id,
    )
    .fetch_optional(&state.db)
    .await?
    .map(|r| DeployInfo {
        hash:        r.commit_hash,
        message:     r.commit_msg,
        deployed_at: r.deployed_at,
        deployed_by: r.deployed_by,
    });

    let hostname = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "localhost".into());

    let remote_url = if site.git_deploy_enabled {
        Some(format!("git@{}:{}", hostname, site_id))
    } else {
        None
    };

    Ok(Json(GitDeployStatus {
        enabled:     site.git_deploy_enabled,
        repo_path:   site.git_repo_path,
        remote_url,
        last_deploy,
    }))
}

/// POST /api/v1/sites/{id}/git-deploy/enable
async fn enable_deploy(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    claims: Claims,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    let remote_url = svc::enable_git_deploy(&state, site_id)
        .await
        .map_err(|e| ApiError::Internal(e))?;

    Ok(Json(serde_json::json!({
        "success":    true,
        "remote_url": remote_url,
        "message":    format!("Run: git remote add jottiecp {}", remote_url)
    })))
}

/// POST /api/v1/sites/{id}/git-deploy/disable
async fn disable_deploy(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    claims: Claims,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    svc::disable_git_deploy(&state, site_id)
        .await
        .map_err(|e| ApiError::Internal(e))?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/sites/{id}/git-deploy/history
async fn get_history(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    claims: Claims,
) -> ApiResult<Json<Vec<DeployInfo>>> {
    require_admin(&claims)?;

    let records = svc::deploy_history(&state, site_id, 10)
        .await
        .map_err(|e| ApiError::Internal(e))?;

    let infos: Vec<DeployInfo> = records
        .into_iter()
        .map(|r| DeployInfo {
            hash:        r.commit_hash,
            message:     r.commit_msg,
            deployed_at: r.deployed_at,
            deployed_by: r.deployed_by,
        })
        .collect();

    Ok(Json(infos))
}

/// POST /api/v1/sites/{id}/git-deploy/trigger  — manually replay the latest push
async fn trigger_deploy(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    claims: Claims,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&claims)?;

    let hash = svc::trigger_deploy(&state, site_id)
        .await
        .map_err(|e| ApiError::Internal(e))?;

    Ok(Json(serde_json::json!({
        "success":     true,
        "commit_hash": hash
    })))
}

/// POST /api/v1/sites/{id}/git-deploy/record
/// Internal endpoint called by the git post-receive hook.
/// Verified by the `ORBIT_INTERNAL_TOKEN` shared secret (Bearer token).
///
/// SECURITY: The token comparison uses a constant-time equality check to
/// prevent timing-based token oracle attacks.  The endpoint also requires
/// the token to be non-empty (panicking at startup is preferable to running
/// with an unauthenticated internal endpoint).
async fn record_deploy_internal(
    State(state): State<Arc<AppState>>,
    Path(site_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RecordDeployRequest>,
) -> ApiResult<StatusCode> {
    // Require a non-empty internal token.  An empty token would allow any request
    // through (empty string == empty string), effectively disabling authentication.
    let expected_token = state.config.internal_token.as_deref().unwrap_or("");
    if expected_token.is_empty() {
        tracing::error!(
            "ORBIT_INTERNAL_TOKEN is not set — /git-deploy/record endpoint is refusing all requests"
        );
        return Err(ApiError::Unauthorized);
    }

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");

    // Constant-time comparison to resist timing oracle attacks on the token.
    let token_matches = {
        let exp = expected_token.as_bytes();
        let got = provided.as_bytes();
        // Use subtle::ConstantTimeEq if available; otherwise fall back to a
        // manual byte-by-byte comparison that reads every byte regardless of
        // mismatch to prevent short-circuit leakage.
        if exp.len() != got.len() {
            false
        } else {
            exp.iter().zip(got.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
        }
    };

    if !token_matches {
        return Err(ApiError::Unauthorized);
    }

    // Validate commit hash is a valid hex SHA (40 chars for SHA-1 or 64 for SHA-256).
    let hash = body.hash.trim();
    let valid_hash = (hash.len() == 40 || hash.len() == 64)
        && hash.chars().all(|c| c.is_ascii_hexdigit());
    if !valid_hash {
        return Err(ApiError::Validation(
            "commit hash must be a 40- or 64-character hex string".into()
        ));
    }

    svc::record_deploy(&state, site_id, hash, body.commit_msg.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e))?;

    Ok(StatusCode::CREATED)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("admin role required".into()));
    }
    Ok(())
}

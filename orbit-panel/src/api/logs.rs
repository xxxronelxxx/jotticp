//! Global log viewer endpoint backing the /logs admin page.
//! GET /api/v1/logs?type=<access|error|php_error|mail|auth|firewall>&site_id=<uuid?>&limit=<n>
//! Returns { "lines": [...] } — the last N lines of the relevant log file.
//! Log paths are whitelisted per type; callers never supply arbitrary paths.

use axum::{
    Router,
    routing::get,
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/v1/logs", get(get_logs))
}

#[derive(Deserialize)]
pub struct GlobalLogsQuery {
    #[serde(default)]
    pub r#type: String,
    pub site_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize { 500 }

#[derive(Serialize)]
pub struct LogsResponse {
    pub lines: Vec<String>,
}

async fn get_logs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<GlobalLogsQuery>,
) -> ApiResult<Json<LogsResponse>> {
    let is_admin = claims.role == "admin";
    let uid: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let n = q.limit.clamp(10, 5000);

    let candidates: Vec<String> = if let Some(site_id) = q.site_id {
        // Site-scoped logs — ownership enforced in SQL (admin sees all).
        let site = sqlx::query!(
            "SELECT unix_user, domain, php_version FROM sites
             WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
            site_id, is_admin, uid
        )
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound { resource: "site" })?;

        match q.r#type.as_str() {
            "error" => vec![
                format!("/home/{}/logs/error.log", site.unix_user),
                format!("/var/log/nginx/{}-error.log", site.domain),
            ],
            "php_error" | "php" => vec![
                format!("/home/{}/logs/php-error.log", site.unix_user),
                format!("/var/log/php{}-fpm.log", site.php_version),
            ],
            _ => vec![
                format!("/home/{}/logs/access.log", site.unix_user),
                format!("/var/log/nginx/{}-access.log", site.domain),
            ],
        }
    } else {
        // System-wide logs — admin only.
        if !is_admin {
            return Err(ApiError::Forbidden("System logs require admin role".into()));
        }
        match q.r#type.as_str() {
            "mail"     => vec!["/var/log/mail.log".to_string()],
            "auth"     => vec!["/var/log/auth.log".to_string()],
            "firewall" => vec!["/var/log/ufw.log".to_string(), "/var/log/fail2ban.log".to_string()],
            "error"    => vec!["/var/log/nginx/error.log".to_string()],
            _          => vec!["/var/log/nginx/access.log".to_string()],
        }
    };

    for path in &candidates {
        if let Ok(out) = tokio::process::Command::new("tail")
            .args(["-n", &n.to_string(), path])
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
                if !lines.is_empty() {
                    return Ok(Json(LogsResponse { lines }));
                }
            }
        }
    }
    // No log file found / empty — return empty set rather than 404 so the UI renders cleanly.
    Ok(Json(LogsResponse { lines: vec![] }))
}

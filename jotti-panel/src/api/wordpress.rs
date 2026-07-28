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

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WpPlugin {
    pub name:    String,
    pub status:  String, // "active" | "inactive" | "must-use" | "dropin"
    pub version: String,
    pub update:  String, // "available" | "none" | "unknown"
    pub title:   String,
}

#[derive(Debug, Serialize)]
pub struct WpTheme {
    pub name:    String,
    pub status:  String,
    pub version: String,
    pub update:  String,
    pub title:   String,
}

#[derive(Debug, Serialize)]
pub struct WpUser {
    pub id:           i64,
    pub user_login:   String,
    pub user_email:   String,
    pub display_name: String,
    pub roles:        String,
}

#[derive(Debug, Serialize)]
pub struct WpCoreStatus {
    pub current_version: String,
    pub latest_version:  String,
    pub update_available: bool,
}

#[derive(Debug, Serialize)]
pub struct WpLoginUrl {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallPluginRequest {
    pub slug:    String,
    pub version: Option<String>,
    pub activate: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ActivatePluginRequest {
    pub activate: bool,
}

#[derive(Debug, Deserialize)]
pub struct InstallThemeRequest {
    pub slug:    String,
    pub version: Option<String>,
    pub activate: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWpUserRequest {
    pub user_login:   String,
    pub user_email:   String,
    pub display_name: Option<String>,
    pub role:         Option<String>, // "administrator" | "editor" | "author" | "subscriber"
    pub password:     Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WpLoginRequest {
    pub user_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Core status & update
        .route("/api/v1/sites/{site_id}/wordpress/core",          get(get_core_status))
        .route("/api/v1/sites/{site_id}/wordpress/core/update",   post(update_core))
        // Plugins
        .route("/api/v1/sites/{site_id}/wordpress/plugins",       get(list_plugins).post(install_plugin))
        .route("/api/v1/sites/{site_id}/wordpress/plugins/{slug}", put(toggle_plugin).delete(delete_plugin))
        .route("/api/v1/sites/{site_id}/wordpress/plugins/{slug}/update", post(update_plugin))
        // Themes
        .route("/api/v1/sites/{site_id}/wordpress/themes",        get(list_themes).post(install_theme))
        .route("/api/v1/sites/{site_id}/wordpress/themes/{slug}/activate", post(activate_theme))
        .route("/api/v1/sites/{site_id}/wordpress/themes/{slug}", delete(delete_theme))
        // Users
        .route("/api/v1/sites/{site_id}/wordpress/users",         get(list_users).post(create_user))
        .route("/api/v1/sites/{site_id}/wordpress/users/{id}/login-url", post(get_login_url))
        // Update all
        .route("/api/v1/sites/{site_id}/wordpress/update-all",    post(update_all))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    Ok((user_id, claims.role == "admin"))
}

/// Resolve the WordPress installation path for a site.
/// Returns (unix_user, wp_path) on success.
async fn resolve_wp(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<(String, String)> {
    let site = sqlx::query!(
        "SELECT unix_user, domain FROM sites
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let docroot = format!("/home/{}/public_html", site.unix_user);
    Ok((site.unix_user, docroot))
}

/// Run a WP-CLI command as the site unix user.
/// Returns parsed JSON value on success.
async fn wp_json(unix_user: &str, wp_path: &str, args: &[&str]) -> ApiResult<Value> {
    validate_unix_user(unix_user)?;

    let mut full_args = vec![
        "sudo", "-u", unix_user, "wp", "--path", wp_path, "--no-color",
    ];
    full_args.extend_from_slice(args);
    full_args.extend_from_slice(&["--format=json"]);

    let out = tokio::process::Command::new(full_args[0])
        .args(&full_args[1..])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("wp-cli spawn: {}", e)))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ApiError::ExternalService(
            format!("wp-cli error: {}", stderr.trim())
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("wp-cli JSON parse: {} (output: {})", e, stdout.trim())))
}

/// Run a WP-CLI command, return raw stdout string.
async fn wp_raw(unix_user: &str, wp_path: &str, args: &[&str]) -> ApiResult<String> {
    validate_unix_user(unix_user)?;

    let mut full_args = vec!["sudo", "-u", unix_user, "wp", "--path", wp_path, "--no-color"];
    full_args.extend_from_slice(args);

    let out = tokio::process::Command::new(full_args[0])
        .args(&full_args[1..])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("wp-cli spawn: {}", e)))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ApiError::ExternalService(
            format!("wp-cli: {}", stderr.trim())
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Validate unix username to prevent shell injection.
fn validate_unix_user(u: &str) -> ApiResult<()> {
    if u.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-') {
        return Err(ApiError::Internal(anyhow::anyhow!("invalid unix user")));
    }
    Ok(())
}

/// Validate plugin/theme slug: alphanumeric + hyphens.
fn validate_slug(slug: &str) -> ApiResult<()> {
    if slug.is_empty() || slug.len() > 200 {
        return Err(ApiError::Validation("slug must be 1-200 characters".into()));
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err(ApiError::Validation("slug contains invalid characters".into()));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/sites/{site_id}/wordpress/core
async fn get_core_status(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<WpCoreStatus>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let current = wp_raw(&unix_user, &wp_path, &["core", "version"]).await
        .unwrap_or_else(|_| "unknown".to_string());

    // `wp core check-update` outputs JSON array of available updates
    let updates = wp_json(&unix_user, &wp_path, &["core", "check-update"]).await
        .unwrap_or(Value::Array(vec![]));

    let (latest, update_available) = if let Value::Array(arr) = &updates {
        if let Some(first) = arr.first() {
            let v = first.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (!v.is_empty(), !v.is_empty())
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    let latest_version = if let (true, Value::Array(arr)) = (latest, &updates) {
        arr.first()
            .and_then(|u| u.get("version"))
            .and_then(|v| v.as_str())
            .unwrap_or(&current)
            .to_string()
    } else {
        current.clone()
    };

    Ok(Json(WpCoreStatus { current_version: current, latest_version, update_available }))
}

/// POST /api/v1/sites/{site_id}/wordpress/core/update
async fn update_core(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<MessageResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;
    wp_raw(&unix_user, &wp_path, &["core", "update"]).await?;
    Ok(Json(MessageResponse { message: "WordPress core updated".to_string() }))
}

/// GET /api/v1/sites/{site_id}/wordpress/plugins
async fn list_plugins(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<WpPlugin>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let data = wp_json(&unix_user, &wp_path, &[
        "plugin", "list",
        "--fields=name,status,version,update,title",
    ]).await?;

    let plugins: Vec<WpPlugin> = if let Value::Array(arr) = data {
        arr.into_iter().filter_map(|v| Some(WpPlugin {
            name:    v.get("name")?.as_str()?.to_string(),
            status:  v.get("status")?.as_str()?.to_string(),
            version: v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            update:  v.get("update").and_then(|x| x.as_str()).unwrap_or("none").to_string(),
            title:   v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })).collect()
    } else {
        vec![]
    };

    Ok(Json(plugins))
}

/// POST /api/v1/sites/{site_id}/wordpress/plugins
async fn install_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<InstallPluginRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&req.slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let mut args = vec!["plugin", "install", req.slug.as_str()];
    let ver;
    if let Some(ref v) = req.version {
        ver = format!("--version={}", v);
        args.push(&ver);
    }
    if req.activate.unwrap_or(false) {
        args.push("--activate");
    }
    wp_raw(&unix_user, &wp_path, &args).await?;

    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("Plugin '{}' installed", req.slug),
    })))
}

/// PUT /api/v1/sites/{site_id}/wordpress/plugins/{slug} — activate or deactivate
async fn toggle_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, slug)): Path<(Uuid, String)>,
    Json(req): Json<ActivatePluginRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let action = if req.activate { "activate" } else { "deactivate" };
    wp_raw(&unix_user, &wp_path, &["plugin", action, &slug]).await?;

    Ok(Json(MessageResponse {
        message: format!("Plugin '{}' {}d", slug, action),
    }))
}

/// POST /api/v1/sites/{site_id}/wordpress/plugins/{slug}/update
async fn update_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, slug)): Path<(Uuid, String)>,
) -> ApiResult<Json<MessageResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;
    wp_raw(&unix_user, &wp_path, &["plugin", "update", &slug]).await?;
    Ok(Json(MessageResponse { message: format!("Plugin '{}' updated", slug) }))
}

/// DELETE /api/v1/sites/{site_id}/wordpress/plugins/{slug}
async fn delete_plugin(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, slug)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;
    wp_raw(&unix_user, &wp_path, &["plugin", "deactivate", &slug]).await.ok();
    wp_raw(&unix_user, &wp_path, &["plugin", "delete", &slug]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/sites/{site_id}/wordpress/themes
async fn list_themes(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<WpTheme>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let data = wp_json(&unix_user, &wp_path, &[
        "theme", "list",
        "--fields=name,status,version,update,title",
    ]).await?;

    let themes: Vec<WpTheme> = if let Value::Array(arr) = data {
        arr.into_iter().filter_map(|v| Some(WpTheme {
            name:    v.get("name")?.as_str()?.to_string(),
            status:  v.get("status")?.as_str()?.to_string(),
            version: v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            update:  v.get("update").and_then(|x| x.as_str()).unwrap_or("none").to_string(),
            title:   v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })).collect()
    } else {
        vec![]
    };

    Ok(Json(themes))
}

/// POST /api/v1/sites/{site_id}/wordpress/themes
async fn install_theme(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<InstallThemeRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&req.slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let mut args = vec!["theme", "install", req.slug.as_str()];
    let ver;
    if let Some(ref v) = req.version {
        ver = format!("--version={}", v);
        args.push(&ver);
    }
    if req.activate.unwrap_or(false) {
        args.push("--activate");
    }
    wp_raw(&unix_user, &wp_path, &args).await?;

    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("Theme '{}' installed", req.slug),
    })))
}

/// POST /api/v1/sites/{site_id}/wordpress/themes/{slug}/activate
async fn activate_theme(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, slug)): Path<(Uuid, String)>,
) -> ApiResult<Json<MessageResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;
    wp_raw(&unix_user, &wp_path, &["theme", "activate", &slug]).await?;
    Ok(Json(MessageResponse { message: format!("Theme '{}' activated", slug) }))
}

/// DELETE /api/v1/sites/{site_id}/wordpress/themes/{slug}
async fn delete_theme(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, slug)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    validate_slug(&slug)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;
    wp_raw(&unix_user, &wp_path, &["theme", "delete", &slug]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/sites/{site_id}/wordpress/users
async fn list_users(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<WpUser>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    let data = wp_json(&unix_user, &wp_path, &[
        "user", "list",
        "--fields=ID,user_login,user_email,display_name,roles",
    ]).await?;

    let users: Vec<WpUser> = if let Value::Array(arr) = data {
        arr.into_iter().filter_map(|v| Some(WpUser {
            id:           v.get("ID")?.as_str()?.parse().unwrap_or(0),
            user_login:   v.get("user_login")?.as_str()?.to_string(),
            user_email:   v.get("user_email").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            display_name: v.get("display_name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            roles:        v.get("roles").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })).collect()
    } else {
        vec![]
    };

    Ok(Json(users))
}

/// POST /api/v1/sites/{site_id}/wordpress/users
async fn create_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<CreateWpUserRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    // Validate inputs
    if req.user_login.len() < 3 || req.user_login.len() > 60 {
        return Err(ApiError::Validation("user_login must be 3-60 characters".into()));
    }
    if !req.user_login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return Err(ApiError::Validation("user_login contains invalid characters".into()));
    }

    let role = req.role.as_deref().unwrap_or("subscriber");
    let allowed_roles = ["administrator", "editor", "author", "contributor", "subscriber"];
    if !allowed_roles.contains(&role) {
        return Err(ApiError::Validation(format!("role must be one of: {}", allowed_roles.join(", "))));
    }

    let mut args = vec![
        "user", "create",
        req.user_login.as_str(),
        req.user_email.as_str(),
        "--role", role,
    ];
    let pw_arg;
    if let Some(ref pw) = req.password {
        pw_arg = format!("--user_pass={}", pw);
        args.push(&pw_arg);
    }
    let dn_arg;
    if let Some(ref dn) = req.display_name {
        dn_arg = format!("--display_name={}", dn);
        args.push(&dn_arg);
    }
    args.push("--porcelain"); // Return only the new user ID

    let new_id = wp_raw(&unix_user, &wp_path, &args).await?;

    Ok((StatusCode::CREATED, Json(json!({
        "id": new_id.trim().parse::<i64>().unwrap_or(0),
        "user_login": req.user_login,
        "user_email": req.user_email,
        "role": role,
    }))))
}

/// POST /api/v1/sites/{site_id}/wordpress/users/{id}/login-url
/// Generate a one-click admin login URL (no password needed).
async fn get_login_url(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, wp_user_id)): Path<(Uuid, i64)>,
) -> ApiResult<Json<WpLoginUrl>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    // `wp user application-password create` or simply reset + return a short-lived token.
    // The cleanest approach: use `wp eval` to call wp_set_password and wp_signon.
    // Simpler: generate a one-time password and return the login URL.
    let tmp_pass: String = Uuid::new_v4().to_string().replace('-', "")[..24].to_string();

    let uid_str = wp_user_id.to_string();
    wp_raw(&unix_user, &wp_path, &[
        "user", "update", &uid_str,
        &format!("--user_pass={}", tmp_pass),
    ]).await?;

    // Build login URL
    let site_url = wp_raw(&unix_user, &wp_path, &["option", "get", "siteurl"]).await
        .unwrap_or_default();
    let site_url = site_url.trim().trim_end_matches('/');

    // Encode the login URL with a nonce via wp-login.php
    let login_url = format!(
        "{}/wp-login.php?log={}&pwd={}&redirect_to=%2Fwp-admin%2F",
        site_url, wp_user_id, tmp_pass
    );

    Ok(Json(WpLoginUrl { url: login_url }))
}

/// POST /api/v1/sites/{site_id}/wordpress/update-all
/// Update all plugins + themes + core in one call.
async fn update_all(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<MessageResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (unix_user, wp_path) = resolve_wp(&state, site_id, user_id, is_admin).await?;

    // Run updates sequentially — failures are non-fatal
    let core_res = wp_raw(&unix_user, &wp_path, &["core", "update"]).await;
    let plugin_res = wp_raw(&unix_user, &wp_path, &["plugin", "update", "--all"]).await;
    let theme_res = wp_raw(&unix_user, &wp_path, &["theme", "update", "--all"]).await;

    let parts: Vec<&str> = [
        if core_res.is_ok() { "core" } else { "" },
        if plugin_res.is_ok() { "plugins" } else { "" },
        if theme_res.is_ok() { "themes" } else { "" },
    ].iter().filter(|s| !s.is_empty()).copied().collect();

    Ok(Json(MessageResponse {
        message: if parts.is_empty() {
            "All components already up to date".to_string()
        } else {
            format!("Updated: {}", parts.join(", "))
        },
    }))
}

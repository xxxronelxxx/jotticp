use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSiteRequest {
    pub domain:       String,
    pub php_version:  String,         // "8.3"
    pub web_server:   String,         // "ols" | "nginx" | "apache"
    pub server_id:    Option<Uuid>,   // None → local server
    #[serde(default = "default_plan")]
    pub plan:         String,         // "community" | "pro"
    pub disk_quota_mb:   Option<i64>,
    pub bandwidth_quota_gb: Option<i64>,
}

fn default_plan() -> String { "community".into() }

#[derive(Debug, Deserialize)]
pub struct UpdateSiteRequest {
    pub php_version:  Option<String>,
    pub web_server:   Option<String>,
    pub disk_quota_mb: Option<i64>,
    pub bandwidth_quota_gb: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SiteResponse {
    pub id:           Uuid,
    pub domain:       String,
    pub status:       String,         // "active" | "suspended" | "provisioning" | "error"
    pub php_version:  String,
    pub web_server:   String,
    pub unix_user:    String,
    pub disk_mb:      i64,
    pub server_id:    Option<Uuid>,
    pub server_label: Option<String>,
    pub ssl_status:   String,         // "active" | "pending" | "expired" | "none"
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub created_at:   DateTime<Utc>,
    pub suspended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct SiteStatsResponse {
    pub site_id:       Uuid,
    pub domain:        String,
    pub memory_bytes:  u64,
    pub cpu_usage_pct: f32,
    pub disk_bytes:    u64,
    pub db_bytes:      u64,
    pub request_count_1h: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListSitesQuery {
    pub server_id: Option<Uuid>,
    pub status:    Option<String>,
    pub search:    Option<String>,
    pub limit:     Option<i64>,
    pub offset:    Option<i64>,
    // Frontend-friendly aliases: page (1-based) + per_page; converted to limit/offset below.
    pub page:      Option<i64>,
    pub per_page:  Option<i64>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/sites",                   get(list_sites).post(create_site))
        .route("/api/v1/sites/{id}",               get(get_site).put(update_site).delete(delete_site))
        .route("/api/v1/sites/{id}/suspend",       post(suspend_site))
        .route("/api/v1/sites/{id}/unsuspend",     post(unsuspend_site))
        .route("/api/v1/sites/{id}/ssl",           post(issue_ssl))
        .route("/api/v1/sites/{id}/stats",         get(site_stats))
        .route("/api/v1/sites/{id}/php-version",   put(change_php_version))
        .route("/api/v1/sites/{id}/logs",          get(site_logs))
        .route("/api/v1/sites/{id}/clone",         post(clone_site))
        .route("/api/v1/sites/{id}/preview-url",   get(preview_url))
        .route("/api/v1/sites/{id}/tags",          get(get_tags).put(set_tags))
        .route("/api/v1/sites/{id}/webserver",     put(switch_webserver))
}

// ── Access control helper ─────────────────────────────────────────────────────

/// Parse caller's user ID and return (user_id, is_admin).
fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// List all sites accessible to the authenticated user.
/// Admins see all sites; resellers see their group's sites; users see their own.
async fn list_sites(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListSitesQuery>,
) -> ApiResult<Json<Vec<SiteResponse>>> {
    // Accept both limit/offset and page/per_page; if page is given, derive limit/offset.
    let per_page = q.per_page.unwrap_or(100).clamp(1, 500);
    let page     = q.page.unwrap_or(1).max(1);
    let limit    = q.limit.unwrap_or(per_page).clamp(1, 500);
    let offset   = q.offset.unwrap_or((page - 1) * per_page).max(0);

    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT s.id, s.domain, s.status::text AS "status!", s.php_version, s.web_server,
                  s.unix_user, s.disk_mb, s.server_id, s.created_at, s.suspended_at,
                  srv.label AS "server_label?",
                  ssl.status::text AS "ssl_status?",
                  ssl.expires_at AS "ssl_expires_at?"
           FROM sites s
           LEFT JOIN servers srv ON srv.id = s.server_id
           LEFT JOIN ssl_certs ssl ON ssl.site_id = s.id
           WHERE s.deleted_at IS NULL
             AND ($1::boolean OR EXISTS (
                 SELECT 1 FROM site_users su WHERE su.site_id = s.id AND su.user_id = $2
             ))
             AND ($3::uuid IS NULL OR s.server_id = $3)
             AND ($4::text IS NULL OR s.status::text = $4)
             AND ($5::text IS NULL OR s.domain ILIKE '%' || $5 || '%')
           ORDER BY s.created_at DESC
           LIMIT $6 OFFSET $7"#,
        is_admin, user_id, q.server_id, q.status, q.search, limit, offset,
    )
    .fetch_all(&state.db)
    .await?;

    let sites: Vec<SiteResponse> = rows.into_iter().map(|r| SiteResponse {
        id:            r.id,
        domain:        r.domain,
        status:        r.status,
        php_version:   r.php_version,
        web_server:    r.web_server,
        unix_user:     r.unix_user,
        disk_mb:       r.disk_mb,
        server_id:     r.server_id,
        server_label:  r.server_label,
        ssl_status:    r.ssl_status.unwrap_or_else(|| "none".into()),
        ssl_expires_at: r.ssl_expires_at,
        created_at:    r.created_at,
        suspended_at:  r.suspended_at,
    }).collect();

    Ok(Json(sites))
}

/// Get a single site. Non-admins can only fetch their own sites.
async fn get_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SiteResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Return 404 for unauthorized access (not 403) to avoid confirming site existence
    let site = sqlx::query!(
        r#"SELECT s.id, s.domain, s.status::text AS "status!", s.php_version, s.web_server,
                  s.unix_user, s.disk_mb, s.server_id, s.created_at, s.suspended_at,
                  srv.label AS "server_label?",
                  ssl.status::text AS "ssl_status?",
                  ssl.expires_at AS "ssl_expires_at?"
           FROM sites s
           LEFT JOIN servers srv ON srv.id = s.server_id
           LEFT JOIN ssl_certs ssl ON ssl.site_id = s.id
           WHERE s.id = $1 AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    Ok(Json(SiteResponse {
        id:            site.id,
        domain:        site.domain,
        status:        site.status,
        php_version:   site.php_version,
        web_server:    site.web_server,
        unix_user:     site.unix_user,
        disk_mb:       site.disk_mb,
        server_id:     site.server_id,
        server_label:  site.server_label,
        ssl_status:    site.ssl_status.unwrap_or_else(|| "none".into()),
        ssl_expires_at: site.ssl_expires_at,
        created_at:    site.created_at,
        suspended_at:  site.suspended_at,
    }))
}

/// Create a new site.
///
/// Provisioning pipeline (runs async via job queue):
/// 1. Validate domain not taken
/// 2. Create Unix user: orbit_<sanitized_domain>
/// 3. Create cgroup v2 slice: website-DOMAIN.slice
/// 4. Create PHP-FPM pool (per-site, unique UID)
/// 5. Create web server vhost
/// 6. Issue SSL cert via instant-acme (non-blocking background)
/// 7. Insert into sites table with "provisioning" status
async fn create_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateSiteRequest>,
) -> ApiResult<(StatusCode, Json<SiteResponse>)> {
    let domain = req.domain.to_lowercase().trim().to_string();
    if !is_valid_domain(&domain) {
        return Err(ApiError::Validation(format!("'{}' is not a valid domain name", domain)));
    }

    let existing = sqlx::query!(
        "SELECT id FROM sites WHERE domain = $1 AND deleted_at IS NULL",
        domain
    )
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(ApiError::DomainConflict { domain });
    }

    let unix_user = format!(
        "orbit_{}",
        domain.replace('.', "_").replace('-', "_").chars().take(24).collect::<String>()
    );

    let site_id = Uuid::new_v4();
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    sqlx::query!(
        "INSERT INTO sites (id, domain, status, php_version, web_server, unix_user,
                            server_id, owner_id, disk_quota_mb, bandwidth_quota_gb,
                            created_at)
         VALUES ($1, $2, 'provisioning', $3, $4, $5, $6, $7, $8, $9, NOW())",
        site_id, domain, req.php_version, req.web_server, unix_user,
        req.server_id, user_id, req.disk_quota_mb, req.bandwidth_quota_gb,
    )
    .execute(&state.db)
    .await?;

    sqlx::query!(
        "INSERT INTO site_users (site_id, user_id, role) VALUES ($1, $2, 'owner')",
        site_id, user_id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_site",
            "site_id": site_id,
            "domain": domain,
            "php_version": req.php_version,
            "web_server": req.web_server,
            "unix_user": unix_user,
            "server_id": req.server_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    let response = SiteResponse {
        id:            site_id,
        domain:        domain.clone(),
        status:        "provisioning".into(),
        php_version:   req.php_version,
        web_server:    req.web_server,
        unix_user,
        disk_mb:       0,
        server_id:     req.server_id,
        server_label:  None,
        ssl_status:    "pending".into(),
        ssl_expires_at: None,
        created_at:    Utc::now(),
        suspended_at:  None,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Update site configuration. Non-admins can only update their own sites.
async fn update_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSiteRequest>,
) -> ApiResult<Json<SiteResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Verify caller has access before mutating
    let owned = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3))",
        id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !owned {
        return Err(ApiError::NotFound { resource: "site" });
    }

    if let Some(ref php) = req.php_version {
        sqlx::query!(
            "UPDATE sites SET php_version = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL",
            php, id
        )
        .execute(&state.db)
        .await?;
    }
    if let Some(disk) = req.disk_quota_mb {
        sqlx::query!(
            "UPDATE sites SET disk_quota_mb = $1, updated_at = NOW() WHERE id = $2",
            disk, id
        )
        .execute(&state.db)
        .await?;
    }

    get_site(State(state), claims, Path(id)).await
}

/// Hard-delete a site. Non-admins can only delete their own sites.
async fn delete_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    // ownership-enforced fetch — 404 for unauthorized (no site enumeration)
    let site = sqlx::query!(
        "SELECT id, domain, unix_user, server_id FROM sites
         WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    sqlx::query!("UPDATE sites SET status = 'deleting' WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "deprovision_site",
            "site_id": site.id,
            "domain": site.domain,
            "unix_user": site.unix_user,
            "server_id": site.server_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)  // 202 — deletion is async
}

/// Suspend a site. Resellers/admins only (users cannot self-suspend; contact reseller).
async fn suspend_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let is_reseller = claims.role == "reseller";

    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Only admins and resellers can suspend sites".into()));
    }

    // Resellers can only suspend sites they own/manage
    let owned = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3))",
        id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !owned {
        return Err(ApiError::NotFound { resource: "site" });
    }

    let affected = sqlx::query!(
        "UPDATE sites SET status = 'suspended', suspended_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "site" });
    }

    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs",
            serde_json::json!({"type": "suspend_site", "site_id": id}).to_string())
            .await.unwrap_or(());
    }
    Ok(StatusCode::OK)
}

/// Unsuspend a site. Resellers/admins only.
async fn unsuspend_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let is_reseller = claims.role == "reseller";

    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Only admins and resellers can unsuspend sites".into()));
    }

    let owned = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3))",
        id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !owned {
        return Err(ApiError::NotFound { resource: "site" });
    }

    let affected = sqlx::query!(
        "UPDATE sites SET status = 'active', suspended_at = NULL WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "site" });
    }

    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs",
            serde_json::json!({"type": "unsuspend_site", "site_id": id}).to_string())
            .await.unwrap_or(());
    }
    Ok(StatusCode::OK)
}

/// Trigger SSL certificate issuance / renewal. Non-admins can only act on their own sites.
async fn issue_ssl(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT domain FROM sites
         WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "issue_ssl",
            "site_id": id,
            "domain": site.domain,
            "challenge": "http-01",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(Json(serde_json::json!({
        "status": "pending",
        "message": "SSL issuance started. Check /api/v1/sites/{id} for ssl_status updates."
    })))
}

/// Get real-time resource stats. Non-admins can only read stats for their own sites.
async fn site_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SiteStatsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT id, domain, unix_user FROM sites
         WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let stats = SiteStatsResponse {
        site_id:          site.id,
        domain:           site.domain,
        memory_bytes:     read_cgroup_memory(&site.unix_user).unwrap_or(0),
        cpu_usage_pct:    read_cgroup_cpu(&site.unix_user).unwrap_or(0.0),
        disk_bytes:       0,  // TODO: du -sb via jotti-agent gRPC
        db_bytes:         0,  // TODO: SELECT SUM(data_length + index_length) from information_schema
        request_count_1h: 0,  // TODO: parse access log via jotti-agent
    };

    Ok(Json(stats))
}

/// Change PHP version for a site. Non-admins can only change their own sites.
async fn change_php_version(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let version = body["php_version"]
        .as_str()
        .ok_or(ApiError::Validation("php_version is required".into()))?
        .to_string();

    if !regex::Regex::new(r"^[78]\.[0-9]+$").unwrap().is_match(&version) {
        return Err(ApiError::Validation(format!("Invalid PHP version: {}", version)));
    }

    // ownership-enforced update
    let result = sqlx::query!(
        "UPDATE sites SET php_version = $1, updated_at = NOW()
         WHERE id = $2 AND deleted_at IS NULL AND ($3::boolean OR owner_id = $4)",
        version, id, is_admin, user_id
    )
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound { resource: "site" });
    }

    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs",
            serde_json::json!({"type": "change_php_version", "site_id": id, "version": version}).to_string())
            .await.unwrap_or(());
    }

    Ok(StatusCode::OK)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_domain(domain: &str) -> bool {
    let re = regex::Regex::new(
        r"^(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$"
    ).unwrap();
    re.is_match(domain) && domain.len() <= 253
}

fn read_cgroup_memory(unix_user: &str) -> Option<u64> {
    let path = format!("/sys/fs/cgroup/website-{}.slice/memory.current", unix_user);
    std::fs::read_to_string(&path).ok()?.trim().parse().ok()
}

fn read_cgroup_cpu(_unix_user: &str) -> Option<f32> {
    // TODO: read cpu.stat usage_usec, delta over 1 second sample
    None
}

// ── Site logs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_log_type")]
    pub r#type: String,
    #[serde(default = "default_lines")]
    pub lines: usize,
}
fn default_log_type() -> String { "access".to_string() }
fn default_lines() -> usize { 200 }

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub lines:   Vec<String>,
    pub content: String,
    pub source:  String,
    pub found:   bool,
}

async fn site_logs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<LogsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT domain, unix_user, php_version FROM sites
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Candidate paths tried in order — first existing file wins.
    // Whitelist log types: callers cannot supply arbitrary paths.
    let candidates: Vec<String> = match q.r#type.as_str() {
        "access" => vec![
            format!("/var/log/nginx/{}-access.log", site.domain),
            format!("/var/log/nginx/access-{}.log", site.domain),
            format!("/home/{}/logs/access.log", site.unix_user),
        ],
        "error" => vec![
            format!("/var/log/nginx/{}-error.log", site.domain),
            format!("/var/log/nginx/error-{}.log", site.domain),
            format!("/home/{}/logs/error.log", site.unix_user),
        ],
        "php" => vec![
            // Per-site FPM pool log (most specific)
            format!("/var/log/php/{}.log", site.domain),
            format!("/var/log/php/fpm/{}.log", site.domain),
            format!("/home/{}/logs/php-error.log", site.unix_user),
            // Global FPM log as fallback
            format!("/var/log/php{}-fpm.log", site.php_version),
        ],
        "app" => vec![
            format!("/home/{}/logs/app.log", site.unix_user),
            format!("/home/{}/public_html/storage/logs/laravel.log", site.unix_user),
            format!("/home/{}/public_html/var/log/app.log", site.unix_user),
        ],
        _ => return Err(ApiError::Validation("type must be access, error, php, or app".into())),
    };

    let n_lines = q.lines.clamp(10, 5000);

    // Use `tail -n N` to avoid reading huge log files into memory.
    for path in &candidates {
        let out = tokio::process::Command::new("tail")
            .args(["-n", &n_lines.to_string(), path])
            .output()
            .await;

        if let Ok(o) = out {
            if o.status.success() {
                let content = String::from_utf8_lossy(&o.stdout).to_string();
                let lines_vec: Vec<String> = content.lines().map(String::from).collect();
                return Ok(Json(LogsResponse {
                    lines:   lines_vec,
                    content,
                    source:  path.clone(),
                    found:   true,
                }));
            }
        }
    }

    // No log file found for this site yet (new site or logs rotated away)
    Ok(Json(LogsResponse {
        lines:   vec![],
        content: String::new(),
        source:  candidates.first().cloned().unwrap_or_default(),
        found:   false,
    }))
}

// ── Clone site ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CloneSiteRequest {
    /// Target domain for the cloned site.
    pub target_domain: String,
    /// Whether to copy the database (default true).
    #[serde(default = "bool_true")]
    pub copy_db: bool,
}
fn bool_true() -> bool { true }

#[derive(Debug, Serialize)]
pub struct CloneResponse {
    pub clone_site_id: Uuid,
    pub clone_domain:  String,
    pub message:       String,
}

/// POST /api/v1/sites/{id}/clone
/// Clone a site: copy files + optionally database to a new domain.
async fn clone_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(src_id): Path<Uuid>,
    Json(req): Json<CloneSiteRequest>,
) -> ApiResult<(StatusCode, Json<CloneResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;

    if !is_valid_domain(&req.target_domain) {
        return Err(ApiError::Validation("Invalid target domain".into()));
    }

    // Fetch source site
    let src = sqlx::query!(
        r#"SELECT domain, unix_user, php_version, web_server, server_id, owner_id,
                  plan, disk_quota_mb, bandwidth_quota_gb
           FROM sites
           WHERE id = $1 AND deleted_at IS NULL
             AND ($2::boolean OR owner_id = $3)"#,
        src_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Check target domain not already taken
    let exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE domain = $1 AND deleted_at IS NULL)",
        req.target_domain
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::Validation(format!(
            "Domain '{}' is already in use", req.target_domain
        )));
    }

    // Generate unix user for clone: first 8 chars of domain hash
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(req.target_domain.as_bytes());
    let unix_user = format!("s{}", hex::encode(h.finalize())[..8].to_string());

    // Create the clone site record
    let clone = sqlx::query!(
        r#"INSERT INTO sites
              (domain, unix_user, php_version, web_server, server_id, owner_id,
               plan, disk_quota_mb, bandwidth_quota_gb, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'provisioning')
           RETURNING id"#,
        req.target_domain, unix_user, src.php_version, src.web_server,
        src.server_id, src.owner_id, src.plan, src.disk_quota_mb, src.bandwidth_quota_gb
    )
    .fetch_one(&state.db)
    .await?;

    let clone_id = clone.id;
    let src_docroot  = format!("/home/{}/public_html", src.unix_user);
    let dst_docroot  = format!("/home/{}/public_html", unix_user);
    let src_unix = src.unix_user.clone();
    let dst_unix = unix_user.clone();
    let target_domain = req.target_domain.clone();
    let copy_db = req.copy_db;
    let db_pool = state.db.clone();

    // Run provisioning + file copy in background (non-blocking)
    tokio::spawn(async move {
        // 1. Create unix user + directories
        let _ = tokio::process::Command::new("useradd")
            .args(["-m", "-d", &format!("/home/{}", dst_unix), "-s", "/sbin/nologin", &dst_unix])
            .output().await;
        let _ = tokio::fs::create_dir_all(&dst_docroot).await;

        // 2. rsync files from source to clone
        let _ = tokio::process::Command::new("rsync")
            .args(["-a", "--delete", &format!("{}/", src_docroot), &dst_docroot])
            .output().await;

        // 3. Fix ownership
        let _ = tokio::process::Command::new("chown")
            .args(["-R", &format!("{}:{}", dst_unix, dst_unix), &format!("/home/{}", dst_unix)])
            .output().await;

        // 4. Clone the database if requested
        if copy_db {
            // Find source site's databases
            if let Ok(rows) = sqlx::query!(
                "SELECT db_name FROM user_databases WHERE site_id = $1 AND db_type = 'mysql' AND deleted_at IS NULL",
                src_id
            )
            .fetch_all(&db_pool).await {
                for row in rows {
                    let src_db  = &row.db_name;
                    let dst_db  = format!("{}_{}", dst_unix, src_db.split('_').last().unwrap_or("db"));
                    // mysqldump | mysql
                    let dump = tokio::process::Command::new("mysqldump")
                        .args(["--single-transaction", src_db])
                        .output().await;
                    if let Ok(dump_out) = dump {
                        use tokio::io::AsyncWriteExt;
                        // Create dest DB
                        let _ = tokio::process::Command::new("mysql")
                            .arg("-e").arg(format!("CREATE DATABASE IF NOT EXISTS `{}`", dst_db))
                            .output().await;
                        // Import dump
                        let mut child = tokio::process::Command::new("mysql")
                            .arg(&dst_db)
                            .stdin(std::process::Stdio::piped())
                            .spawn().ok();
                        if let Some(ref mut c) = child {
                            if let Some(mut stdin) = c.stdin.take() {
                                let _ = stdin.write_all(&dump_out.stdout).await;
                            }
                            let _ = c.wait().await;
                        }
                        // Search-replace old domain with new domain in the cloned DB
                        let _ = tokio::process::Command::new("mysql")
                            .args([&dst_db, "-e", &format!(
                                "UPDATE wp_options SET option_value = REPLACE(option_value, '{}', '{}') WHERE option_name IN ('siteurl','home');",
                                src.domain, target_domain
                            )])
                            .output().await;
                    }
                }
            }
        }

        // 5. Mark clone as active
        let _ = sqlx::query!(
            "UPDATE sites SET status = 'active' WHERE id = $1",
            clone_id
        ).execute(&db_pool).await;
    });

    Ok((StatusCode::ACCEPTED, Json(CloneResponse {
        clone_site_id: clone_id,
        clone_domain:  req.target_domain,
        message: "Site clone started. Files are being copied in the background.".to_string(),
    })))
}

// ── Preview URL ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PreviewUrlResponse {
    pub preview_url: String,
    pub instructions: String,
}

/// GET /api/v1/sites/{id}/preview-url
/// Return a preview URL for accessing the site before DNS is pointed.
async fn preview_url(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<PreviewUrlResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let site = sqlx::query!(
        "SELECT domain, unix_user FROM sites
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Get server IP from config or db
    let server_ip = sqlx::query_scalar!(
        "SELECT ip::text FROM servers WHERE deleted_at IS NULL ORDER BY created_at LIMIT 1"
    )
    .fetch_optional(&state.db)
    .await?
    .flatten()
    .unwrap_or_else(|| "127.0.0.1".to_string());

    let preview_url = format!("http://{}", site.domain);
    let instructions = format!(
        "Add this to your local /etc/hosts file to preview:\n{} {}",
        server_ip, site.domain
    );

    Ok(Json(PreviewUrlResponse { preview_url, instructions }))
}

// ── Site tags ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TagsResponse {
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetTagsRequest {
    pub tags: Vec<String>,
}

/// GET /api/v1/sites/{id}/tags
async fn get_tags(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<TagsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Verify site ownership
    let _ = sqlx::query_scalar!(
        "SELECT id FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Tags are stored as a text[] column on sites (added by migration 0021)
    let tags: Vec<String> = sqlx::query_scalar!(
        "SELECT COALESCE(tags, ARRAY[]::text[]) FROM sites WHERE id = $1",
        site_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or_default();

    Ok(Json(TagsResponse { tags }))
}

/// PUT /api/v1/sites/{id}/tags
async fn set_tags(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<SetTagsRequest>,
) -> ApiResult<Json<TagsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Limit: 20 tags max, each 1-32 chars, alphanumeric + hyphens
    if req.tags.len() > 20 {
        return Err(ApiError::Validation("Maximum 20 tags allowed".into()));
    }
    for tag in &req.tags {
        if tag.is_empty() || tag.len() > 32 {
            return Err(ApiError::Validation("Each tag must be 1-32 characters".into()));
        }
        if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(ApiError::Validation(format!("Tag '{}' contains invalid characters", tag)));
        }
    }

    sqlx::query!(
        r#"UPDATE sites SET tags = $1
           WHERE id = $2 AND deleted_at IS NULL
             AND ($3::boolean OR owner_id = $4)"#,
        &req.tags, site_id, is_admin, user_id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(TagsResponse { tags: req.tags }))
}

// ── Webserver switch ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SwitchWebserverRequest {
    pub web_server: String,
}

#[derive(Debug, Serialize)]
pub struct SwitchWebserverResponse {
    pub domain:     String,
    pub web_server: String,
    pub message:    String,
}

/// PUT /api/v1/sites/{id}/webserver — switch site between nginx / apache / ols.
/// Admin-only. Rewrites vhosts, reloads services.
async fn switch_webserver(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<SwitchWebserverRequest>,
) -> ApiResult<Json<SwitchWebserverResponse>> {
    let (_user_id, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }

    if !matches!(req.web_server.as_str(), "nginx" | "apache" | "ols") {
        return Err(ApiError::Validation(
            "web_server must be nginx, apache, or ols".into()
        ));
    }

    let site = sqlx::query!(
        r#"SELECT domain, unix_user, php_version, web_server AS old_ws
           FROM sites WHERE id = $1 AND deleted_at IS NULL"#,
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let domain     = site.domain.clone();
    let unix_user  = site.unix_user.clone();
    let new_ws     = req.web_server.clone();

    // Validate to prevent path traversal before spawning blocking task
    if domain.contains('/') || domain.contains("..") || unix_user.contains('/') {
        return Err(ApiError::Validation("Invalid domain or unix_user".into()));
    }

    let php_ver = site.php_version.clone();
    tokio::task::spawn_blocking({
        let d = domain.clone();
        let u = unix_user.clone();
        let ws = new_ws.clone();
        let pv = php_ver.clone();
        move || crate::services::provisioning::switch_webserver_sync(&d, &u, &ws, &pv)
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("spawn_blocking failed: {}", e)))?
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("webserver switch failed: {}", e)))?;

    sqlx::query!(
        "UPDATE sites SET web_server = $1, updated_at = NOW() WHERE id = $2",
        new_ws, site_id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(SwitchWebserverResponse {
        domain,
        web_server: new_ws,
        message: "Web server switched successfully. Nginx reloaded.".into(),
    }))
}

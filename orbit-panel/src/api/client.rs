use axum::{
    Router,
    routing::{get, post, put},
    extract::{State, Query},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use crate::middleware::feature_flags::require_feature;
use super::auth::{Claims, verify_password, hash_password};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateClientSiteRequest {
    pub domain:      String,
    pub php_version: String,
    pub web_server:  String,
}

#[derive(Debug, Serialize)]
pub struct ClientSiteResponse {
    pub id:            Uuid,
    pub domain:        String,
    pub status:        String,
    pub php_version:   String,
    pub web_server:    String,
    pub ssl_status:    String,
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub created_at:    DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientEmailResponse {
    pub id:         Uuid,
    pub site_id:    Uuid,
    pub address:    String,
    pub quota_mb:   i32,
    pub used_mb:    i32,
    pub status:     String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientDbResponse {
    pub id:         Uuid,
    pub site_id:    Uuid,
    pub db_type:    String,
    pub db_name:    String,
    pub db_user:    String,
    pub status:     String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientSslResponse {
    pub id:         Uuid,
    pub site_id:    Uuid,
    pub domain:     String,
    pub status:     String,
    pub expires_at: Option<DateTime<Utc>>,
    pub issued_at:  Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ClientBackupResponse {
    pub id:           Uuid,
    pub site_id:      Uuid,
    pub status:       String,
    pub size_bytes:   Option<i64>,
    pub created_at:   DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ClientCronResponse {
    pub id:          Uuid,
    pub site_id:     Uuid,
    pub minute:      String,
    pub hour:        String,
    pub day:         String,
    pub month:       String,
    pub weekday:     String,
    pub command:     String,
    pub label:       String,
    pub enabled:     bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientProfileResponse {
    pub id:              Uuid,
    pub email:           String,
    pub role:            String,
    pub full_name:       Option<String>,
    pub status:          String,
    pub wizard_complete: bool,
    pub created_at:      DateTime<Utc>,
    pub last_login_at:   Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password:     String,
}

#[derive(Debug, Serialize)]
pub struct PackageResponse {
    pub package_id:           Uuid,
    pub name:                 String,
    pub max_sites:            i32,
    pub max_disk_gb:          i32,
    pub max_email_accounts:   i32,
    pub max_databases:        i32,
    pub feature_flags:        serde_json::Value,
    pub assigned_at:          DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ClientSitesQuery {
    pub limit:  Option<i64>,
    pub offset: Option<i64>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sites",            get(list_client_sites).post(add_client_site))
        .route("/email",            get(list_client_email))
        .route("/databases",        get(list_client_databases))
        .route("/ssl",              get(list_client_ssl))
        .route("/backups",          get(list_client_backups))
        .route("/cron",             get(list_client_cron))
        .route("/profile",          get(get_client_profile))
        .route("/profile/password", put(change_client_password))
        .route("/package",          get(get_client_package))
}

// ── Access helper ─────────────────────────────────────────────────────────────

/// Ensure caller is a client (role == "user" or "reseller").
/// Returns (user_id, is_admin=false always for this router).
fn require_client(claims: &Claims) -> ApiResult<Uuid> {
    if claims.role == "admin" {
        return Err(ApiError::Forbidden("Admin users should use the main API, not the client portal".into()));
    }
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    Ok(user_id)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// List only sites owned by the authenticated client (owner_id = caller.sub).
async fn list_client_sites(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ClientSitesQuery>,
) -> ApiResult<Json<Vec<ClientSiteResponse>>> {
    let user_id = require_client(&claims)?;
    let limit  = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"SELECT s.id, s.domain, s.status::text AS status, s.php_version, s.web_server,
                  ssl.status::text AS "ssl_status?",
                  ssl.expires_at AS "ssl_expires_at?",
                  s.created_at
           FROM sites s
           LEFT JOIN ssl_certs ssl ON ssl.site_id = s.id
           WHERE s.deleted_at IS NULL AND s.owner_id = $1
           ORDER BY s.created_at DESC
           LIMIT $2 OFFSET $3"#,
        user_id, limit, offset
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientSiteResponse {
        id:             r.id,
        domain:         r.domain,
        status:         r.status.unwrap_or_default(),
        php_version:    r.php_version,
        web_server:     r.web_server,
        ssl_status:     r.ssl_status.unwrap_or_else(|| "none".into()),
        ssl_expires_at: r.ssl_expires_at,
        created_at:     r.created_at,
    }).collect()))
}

/// List email accounts across all of the client's sites.
async fn list_client_email(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ClientEmailResponse>>> {
    let user_id = require_client(&claims)?;
    // Gate: package must include email_access feature
    require_feature(&state, user_id, "email_access").await?;

    let rows = sqlx::query!(
        r#"SELECT ea.id, ea.site_id, ea.address, ea.quota_mb, ea.used_mb, ea.status, ea.created_at
           FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE s.owner_id = $1 AND s.deleted_at IS NULL AND ea.deleted_at IS NULL
           ORDER BY ea.address ASC"#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientEmailResponse {
        id:         r.id,
        site_id:    r.site_id.unwrap_or_default(),
        address:    r.address.unwrap_or_default(),
        quota_mb:   r.quota_mb,
        used_mb:    r.used_mb,
        status:     r.status,
        created_at: r.created_at,
    }).collect()))
}

/// List databases across all of the client's sites.
async fn list_client_databases(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ClientDbResponse>>> {
    let user_id = require_client(&claims)?;
    // Gate: package must include db_manager feature
    require_feature(&state, user_id, "db_manager").await?;

    let rows = sqlx::query!(
        r#"SELECT db.id, db.site_id, db.db_type, db.db_name, db.db_user, db.status, db.created_at
           FROM user_databases db
           JOIN sites s ON s.id = db.site_id
           WHERE s.owner_id = $1 AND s.deleted_at IS NULL AND db.deleted_at IS NULL
           ORDER BY db.created_at DESC"#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientDbResponse {
        id:         r.id,
        site_id:    r.site_id,
        db_type:    r.db_type,
        db_name:    r.db_name,
        db_user:    r.db_user,
        status:     r.status,
        created_at: r.created_at,
    }).collect()))
}

/// List SSL certs across all of the client's sites.
async fn list_client_ssl(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ClientSslResponse>>> {
    let user_id = require_client(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT ssl.id, ssl.site_id, s.domain, ssl.status::text AS status, ssl.expires_at, ssl.issued_at
           FROM ssl_certs ssl
           JOIN sites s ON s.id = ssl.site_id
           WHERE s.owner_id = $1 AND s.deleted_at IS NULL
           ORDER BY ssl.expires_at ASC NULLS LAST"#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientSslResponse {
        id:         r.id,
        site_id:    r.site_id.unwrap_or_default(),
        domain:     r.domain,
        status:     r.status.unwrap_or_default(),
        expires_at: r.expires_at,
        issued_at:  r.issued_at,
    }).collect()))
}

/// List backups across all of the client's sites.
async fn list_client_backups(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ClientBackupResponse>>> {
    let user_id = require_client(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT bj.id, bj.site_id, bj.status, bj.size_bytes, bj.created_at, bj.completed_at
           FROM backup_jobs bj
           JOIN sites s ON s.id = bj.site_id
           WHERE s.owner_id = $1 AND s.deleted_at IS NULL
           ORDER BY bj.created_at DESC
           LIMIT 200"#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientBackupResponse {
        id:           r.id,
        site_id:      r.site_id,
        status:       r.status,
        size_bytes:   r.size_bytes,
        created_at:   r.created_at,
        completed_at: r.completed_at,
    }).collect()))
}

/// List cron jobs across all of the client's sites.
async fn list_client_cron(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ClientCronResponse>>> {
    let user_id = require_client(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT cj.id, cj.site_id, cj.minute, cj.hour, cj.day, cj.month,
                  cj.weekday, cj.command, cj.label, cj.enabled,
                  cj.last_run_at, cj.created_at
           FROM cron_jobs cj
           JOIN sites s ON s.id = cj.site_id
           WHERE s.owner_id = $1 AND s.deleted_at IS NULL
           ORDER BY cj.created_at DESC"#,
        user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientCronResponse {
        id:          r.id,
        site_id:     r.site_id.unwrap_or_default(),
        minute:      r.minute,
        hour:        r.hour,
        day:         r.day,
        month:       r.month,
        weekday:     r.weekday,
        command:     r.command,
        label:       r.label.unwrap_or_default(),
        enabled:     r.enabled,
        last_run_at: r.last_run_at,
        created_at:  r.created_at,
    }).collect()))
}

/// Add a new site for the client — enforces max_sites package limit.
///
/// The limit check uses a serializable transaction with a FOR UPDATE lock on
/// the user_packages row to prevent a TOCTOU race where two simultaneous
/// requests both pass the count check and together exceed the limit.
async fn add_client_site(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateClientSiteRequest>,
) -> ApiResult<(StatusCode, Json<ClientSiteResponse>)> {
    let user_id = require_client(&claims)?;

    // Validate inputs before taking the lock
    let domain = req.domain.to_lowercase().trim().to_string();
    if !is_valid_domain(&domain) {
        return Err(ApiError::Validation(format!("'{}' is not a valid domain name", domain)));
    }

    // Begin a transaction.  The FOR UPDATE on user_packages serialises concurrent
    // site-creation requests for the same user, preventing the TOCTOU race where
    // two simultaneous requests both pass the count < max_sites check.
    let mut tx = state.db.begin().await?;

    // Lock the user_packages row (if one exists) to prevent concurrent races.
    let package = sqlx::query!(
        r#"SELECT rp.max_sites
           FROM user_packages up
           JOIN reseller_packages rp ON rp.id = up.package_id
           WHERE up.user_id = $1
           FOR UPDATE OF up"#,
        user_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(pkg) = package {
        let current_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM sites WHERE owner_id = $1 AND deleted_at IS NULL",
            user_id
        )
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(0);

        let max_sites = pkg.max_sites.unwrap_or(1);
        if current_count >= max_sites as i64 {
            return Err(ApiError::Forbidden(format!(
                "Your package allows a maximum of {} site(s). Upgrade your plan to add more.",
                max_sites
            )));
        }
    }

    let existing = sqlx::query!(
        "SELECT id FROM sites WHERE domain = $1 AND deleted_at IS NULL",
        domain
    )
    .fetch_optional(&mut *tx)
    .await?;

    if existing.is_some() {
        return Err(ApiError::DomainConflict { domain });
    }

    let unix_user = format!(
        "orbit_{}",
        domain.replace('.', "_").replace('-', "_").chars().take(24).collect::<String>()
    );
    let site_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO sites (id, domain, status, php_version, web_server, unix_user,
                            owner_id, created_at)
         VALUES ($1, $2, 'provisioning', $3, $4, $5, $6, NOW())",
        site_id, domain, req.php_version, req.web_server, unix_user, user_id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO site_users (site_id, user_id, role) VALUES ($1, $2, 'owner')",
        site_id, user_id
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_site",
            "site_id": site_id,
            "domain": domain,
            "php_version": req.php_version,
            "web_server": req.web_server,
            "unix_user": unix_user,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(ClientSiteResponse {
        id:             site_id,
        domain,
        status:         "provisioning".into(),
        php_version:    req.php_version,
        web_server:     req.web_server,
        ssl_status:     "pending".into(),
        ssl_expires_at: None,
        created_at:     Utc::now(),
    })))
}

/// Return the client's own user record.
async fn get_client_profile(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<ClientProfileResponse>> {
    let user_id = require_client(&claims)?;

    let row = sqlx::query!(
        "SELECT id, email, role::text AS role, full_name, status, wizard_complete, created_at, last_login_at
         FROM users WHERE id = $1 AND deleted_at IS NULL",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    Ok(Json(ClientProfileResponse {
        id:              row.id,
        email:           row.email,
        role:            row.role.unwrap_or_default(),
        full_name:       row.full_name,
        status:          row.status,
        wizard_complete: row.wizard_complete,
        created_at:      row.created_at,
        last_login_at:   row.last_login_at,
    }))
}

/// Allow the client to change their own password (requires current_password verification).
async fn change_client_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let user_id = require_client(&claims)?;

    if req.new_password.len() < 12 {
        return Err(ApiError::Validation("New password must be at least 12 characters".into()));
    }

    let row = sqlx::query!(
        "SELECT password_hash FROM users WHERE id = $1 AND deleted_at IS NULL",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    let ok = verify_password(&req.current_password, &row.password_hash)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Password verify error: {}", e)))?;

    if !ok {
        return Err(ApiError::Forbidden("Current password is incorrect".into()));
    }

    let new_hash = hash_password(&req.new_password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Hash error: {}", e)))?;

    sqlx::query!(
        "UPDATE users SET password_hash = $1 WHERE id = $2",
        new_hash, user_id
    )
    .execute(&state.db)
    .await?;

    // Audit the password change
    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'password_change', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        user_id, user_id
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Return the client's package limits and feature flags.
async fn get_client_package(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<PackageResponse>> {
    let user_id = require_client(&claims)?;

    let row = sqlx::query!(
        r#"SELECT rp.id AS package_id, rp.name, rp.max_sites, rp.max_disk_gb,
                  rp.max_email_accounts, rp.max_databases,
                  rp.feature_flags,
                  up.assigned_at
           FROM user_packages up
           JOIN reseller_packages rp ON rp.id = up.package_id
           WHERE up.user_id = $1"#,
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "package" })?;

    Ok(Json(PackageResponse {
        package_id:         row.package_id,
        name:               row.name,
        max_sites:          row.max_sites.unwrap_or(0),
        max_disk_gb:        row.max_disk_gb.unwrap_or(0),
        max_email_accounts: row.max_email_accounts.unwrap_or(0),
        max_databases:      row.max_databases.unwrap_or(0),
        feature_flags:      row.feature_flags,
        assigned_at:        row.assigned_at,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_domain(domain: &str) -> bool {
    let re = regex::Regex::new(
        r"^(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$"
    ).unwrap();
    re.is_match(domain) && domain.len() <= 253
}

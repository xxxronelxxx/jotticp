use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::{Claims, encode_jwt, hash_password};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListClientsQuery {
    pub search: Option<String>,
    pub limit:  Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClientRequest {
    pub email:     String,
    pub password:  String,
    pub full_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientResponse {
    pub id:              Uuid,
    pub email:           String,
    pub role:            String,
    pub full_name:       Option<String>,
    pub status:          String,
    pub reseller_id:     Option<Uuid>,
    pub created_at:      DateTime<Utc>,
    pub last_login_at:   Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ClientListEntry {
    pub id:           Uuid,
    pub email:        String,
    pub display_name: Option<String>,
    pub sites_count:  i64,
    pub package_name: Option<String>,
    pub status:       String,
    pub created_at:   DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientSiteResponse {
    pub id:          Uuid,
    pub domain:      String,
    pub status:      String,
    pub php_version: String,
    pub web_server:  String,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePackageRequest {
    pub name:                 String,
    pub max_sites:            i32,
    pub max_disk_gb:          i32,
    pub max_email_accounts:   i32,
    pub max_databases:        i32,
    pub price_monthly:        f64,
    pub feature_flags:        Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePackageRequest {
    pub name:               Option<String>,
    pub max_sites:          Option<i32>,
    pub max_disk_gb:        Option<i32>,
    pub max_email_accounts: Option<i32>,
    pub max_databases:      Option<i32>,
    pub price_monthly:      Option<f64>,
    pub feature_flags:      Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct PackageResponse {
    pub id:                   Uuid,
    pub reseller_id:          Option<Uuid>,
    pub name:                 String,
    pub max_sites:            i32,
    pub max_disk_gb:          i32,
    pub max_email_accounts:   i32,
    pub max_databases:        i32,
    pub price_monthly:        f64,
    pub features:             serde_json::Value,
    pub clients_count:        i64,
    pub created_at:           DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AssignPackageRequest {
    pub package_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ResellerStatsResponse {
    pub total_clients:          i64,
    pub active_clients:         i64,
    pub total_sites:            i64,
    pub total_disk_used_gb:     f64,
    pub total_revenue_monthly:  f64,
}

#[derive(Debug, Serialize)]
pub struct LoginAsResponse {
    pub access_token:    String,
    pub user_id:         String,
    pub email:           String,
    pub role:            String,
    pub expires_in:      i64,
    pub wizard_complete: bool,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/clients",                      get(list_clients).post(create_client))
        .route("/clients/{id}",                  delete(delete_client))
        .route("/clients/{id}/sites",            get(list_client_sites))
        .route("/clients/{id}/assign-package",   post(assign_package))
        .route("/clients/{id}/suspend",          post(suspend_client))
        .route("/clients/{id}/unsuspend",        post(unsuspend_client))
        .route("/clients/{id}/login-as",         post(login_as_client))
        .route("/packages",                     get(list_packages).post(create_package))
        .route("/packages/{id}",                 put(update_package).delete(delete_package))
        .route("/stats",                        get(reseller_stats))
}

// ── Access helper ─────────────────────────────────────────────────────────────

/// Returns (caller_id, is_admin). Only admin or reseller may call these routes.
fn require_admin_or_reseller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    let is_reseller = claims.role == "reseller";
    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Admin or reseller role required".into()));
    }
    Ok((user_id, is_admin))
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Admin sees all clients (role='user'); reseller sees only their own clients.
async fn list_clients(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListClientsQuery>,
) -> ApiResult<Json<Vec<ClientListEntry>>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;
    let limit  = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"SELECT u.id, u.email, u.full_name AS display_name, u.status, u.created_at,
                  COALESCE(sc.cnt, 0) AS "sites_count!: i64",
                  rp.name AS "package_name?: String"
           FROM users u
           LEFT JOIN (
               SELECT owner_id, COUNT(*) AS cnt FROM sites WHERE deleted_at IS NULL GROUP BY owner_id
           ) sc ON sc.owner_id = u.id
           LEFT JOIN user_packages up ON up.user_id = u.id
           LEFT JOIN reseller_packages rp ON rp.id = up.package_id
           WHERE u.deleted_at IS NULL AND u.role != 'admin'::user_role
             AND ($1::boolean OR u.reseller_id = $2)
             AND ($3::text IS NULL OR u.email ILIKE '%' || $3 || '%'
                  OR u.full_name ILIKE '%' || $3 || '%')
           ORDER BY u.created_at DESC
           LIMIT $4 OFFSET $5"#,
        is_admin, caller_id, q.search, limit, offset
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientListEntry {
        id:           r.id,
        email:        r.email,
        display_name: r.display_name,
        sites_count:  r.sites_count,
        package_name: r.package_name,
        status:       r.status,
        created_at:   r.created_at,
    }).collect()))
}

/// Reseller creates a client under themselves (role='user', reseller_id=caller.sub).
async fn create_client(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateClientRequest>,
) -> ApiResult<(StatusCode, Json<ClientResponse>)> {
    let (caller_id, _is_admin) = require_admin_or_reseller(&claims)?;

    if req.password.len() < 12 {
        return Err(ApiError::Validation("Password must be at least 12 characters".into()));
    }

    let email = req.email.to_lowercase().trim().to_string();
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1 AND deleted_at IS NULL)",
        email
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::Validation(format!("Email {} is already registered", email)));
    }

    let password_hash = hash_password(&req.password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Password hash failed: {}", e)))?;

    let client_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO users (id, email, password_hash, role, full_name, status,
                            wizard_complete, reseller_id, created_at)
         VALUES ($1, $2, $3, 'user', $4, 'active', false, $5, NOW())",
        client_id, email, password_hash, req.full_name, caller_id
    )
    .execute(&state.db)
    .await?;

    // Audit
    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'client_create', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        caller_id, client_id
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(ClientResponse {
        id:            client_id,
        email,
        role:          "user".into(),
        full_name:     req.full_name,
        status:        "active".into(),
        reseller_id:   Some(caller_id),
        created_at:    Utc::now(),
        last_login_at: None,
    })))
}

/// List sites belonging to a specific client. Reseller can only see their own clients.
async fn list_client_sites(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ClientSiteResponse>>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    // Verify the target client belongs to this reseller (or caller is admin)
    let _ = sqlx::query!(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL
         AND ($2::boolean OR reseller_id = $3)",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "client" })?;

    let rows = sqlx::query!(
        r#"SELECT id, domain, status::text as "status", php_version, web_server, created_at
           FROM sites
           WHERE owner_id = $1 AND deleted_at IS NULL
           ORDER BY created_at DESC"#,
        client_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ClientSiteResponse {
        id:          r.id,
        domain:      r.domain,
        status:      r.status.unwrap_or_default(),
        php_version: r.php_version,
        web_server:  r.web_server,
        created_at:  r.created_at,
    }).collect()))
}

/// Create a new hosting package.
/// Admin can create global packages (reseller_id = NULL) or assign to a reseller.
/// Reseller creates their own packages (reseller_id = caller.sub).
async fn create_package(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreatePackageRequest>,
) -> ApiResult<(StatusCode, Json<PackageResponse>)> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("Package name is required".into()));
    }
    if req.max_sites < 1 {
        return Err(ApiError::Validation("max_sites must be at least 1".into()));
    }

    let default_flags = serde_json::json!({
        "dns_editing": true,
        "php_version_switch": true,
        "app_installer": true,
        "ssh_access": false,
        "staging": false,
        "db_manager": true,
        "file_manager": true
    });
    let feature_flags = req.feature_flags.unwrap_or(default_flags);

    let pkg_id = Uuid::new_v4();
    // reseller_id is NOT NULL in DB — admin-created packages use their own user_id as owner
    let reseller_id = caller_id;
    let price_decimal = rust_decimal::Decimal::from_f64(req.price_monthly)
        .unwrap_or_default();

    sqlx::query!(
        r#"INSERT INTO reseller_packages
               (id, reseller_id, name, max_sites, max_disk_gb, max_email_accounts,
                max_databases, price_monthly, feature_flags, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())"#,
        pkg_id, reseller_id, req.name.trim(),
        req.max_sites, req.max_disk_gb, req.max_email_accounts,
        req.max_databases, price_decimal, feature_flags
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(PackageResponse {
        id:                   pkg_id,
        reseller_id:          Some(reseller_id),
        name:                 req.name,
        max_sites:            req.max_sites,
        max_disk_gb:          req.max_disk_gb,
        max_email_accounts:   req.max_email_accounts,
        max_databases:        req.max_databases,
        price_monthly:        req.price_monthly,
        features:             feature_flags,
        clients_count:        0,
        created_at:           Utc::now(),
    })))
}

/// List packages. Admin sees all; reseller sees their own only.
async fn list_packages(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<PackageResponse>>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT rp.id, rp.reseller_id, rp.name, rp.max_sites, rp.max_disk_gb, rp.max_email_accounts,
                  rp.max_databases, rp.price_monthly, rp.feature_flags, rp.created_at,
                  COALESCE(cc.cnt, 0) AS "clients_count!: i64"
           FROM reseller_packages rp
           LEFT JOIN (
               SELECT package_id, COUNT(*) AS cnt FROM user_packages GROUP BY package_id
           ) cc ON cc.package_id = rp.id
           WHERE ($1::boolean OR rp.reseller_id = $2)
           ORDER BY rp.created_at DESC"#,
        is_admin, caller_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| PackageResponse {
        id:                   r.id,
        reseller_id:          Some(r.reseller_id),
        name:                 r.name,
        max_sites:            r.max_sites.unwrap_or(0),
        max_disk_gb:          r.max_disk_gb.unwrap_or(0),
        max_email_accounts:   r.max_email_accounts.unwrap_or(0),
        max_databases:        r.max_databases.unwrap_or(0),
        price_monthly:        r.price_monthly.to_f64().unwrap_or(0.0),
        features:             r.feature_flags,
        clients_count:        r.clients_count,
        created_at:           r.created_at,
    }).collect()))
}

/// Update package limits. Only the owning reseller (or admin) can update.
async fn update_package(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(pkg_id): Path<Uuid>,
    Json(req): Json<UpdatePackageRequest>,
) -> ApiResult<Json<PackageResponse>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    // Ownership check
    let pkg = sqlx::query!(
        r#"SELECT id, reseller_id, name, max_sites, max_disk_gb, max_email_accounts,
                  max_databases, price_monthly, feature_flags, created_at
           FROM reseller_packages
           WHERE id = $1 AND ($2::boolean OR reseller_id = $3)"#,
        pkg_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "package" })?;

    let name             = req.name.as_deref().unwrap_or(&pkg.name).trim().to_string();
    let max_sites        = req.max_sites.unwrap_or(pkg.max_sites.unwrap_or(1));
    let max_disk_gb      = req.max_disk_gb.unwrap_or(pkg.max_disk_gb.unwrap_or(10));
    let max_email        = req.max_email_accounts.unwrap_or(pkg.max_email_accounts.unwrap_or(5));
    let max_db           = req.max_databases.unwrap_or(pkg.max_databases.unwrap_or(5));
    let price_monthly_f64 = req.price_monthly
        .unwrap_or_else(|| pkg.price_monthly.to_f64().unwrap_or(0.0));
    let price_monthly_decimal = rust_decimal::Decimal::from_f64(price_monthly_f64)
        .unwrap_or_default();
    let feature_flags    = req.feature_flags.unwrap_or(pkg.feature_flags.clone());

    sqlx::query!(
        r#"UPDATE reseller_packages
           SET name = $1, max_sites = $2, max_disk_gb = $3, max_email_accounts = $4,
               max_databases = $5, price_monthly = $6, feature_flags = $7
           WHERE id = $8"#,
        name, max_sites, max_disk_gb, max_email, max_db, price_monthly_decimal, feature_flags, pkg_id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(PackageResponse {
        id:                   pkg.id,
        reseller_id:          Some(pkg.reseller_id),
        name,
        max_sites,
        max_disk_gb,
        max_email_accounts:   max_email,
        max_databases:        max_db,
        price_monthly:        price_monthly_f64,
        features:             feature_flags,
        clients_count:        0,
        created_at:           pkg.created_at,
    }))
}

/// Delete a package — only if no active subscribers.
async fn delete_package(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(pkg_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    // Ownership check
    let _ = sqlx::query!(
        "SELECT id FROM reseller_packages WHERE id = $1 AND ($2::boolean OR reseller_id = $3)",
        pkg_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "package" })?;

    // Check for active subscribers
    let subscriber_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_packages WHERE package_id = $1",
        pkg_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if subscriber_count > 0 {
        return Err(ApiError::Validation(format!(
            "Cannot delete package: {} active subscriber(s) are assigned to it",
            subscriber_count
        )));
    }

    sqlx::query!("DELETE FROM reseller_packages WHERE id = $1", pkg_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Assign a package to a client and record it in user_packages.
async fn assign_package(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
    Json(req): Json<AssignPackageRequest>,
) -> ApiResult<StatusCode> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    // Verify client belongs to this reseller
    let _ = sqlx::query!(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL
         AND ($2::boolean OR reseller_id = $3)",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "client" })?;

    // Verify package belongs to this reseller (or is global)
    let _ = sqlx::query!(
        "SELECT id FROM reseller_packages WHERE id = $1
         AND ($2::boolean OR reseller_id = $3 OR reseller_id IS NULL)",
        req.package_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "package" })?;

    // Upsert: update package if already assigned, otherwise insert
    sqlx::query!(
        r#"INSERT INTO user_packages (user_id, package_id, assigned_at)
           VALUES ($1, $2, NOW())
           ON CONFLICT (user_id) DO UPDATE
           SET package_id = EXCLUDED.package_id,
               assigned_at = NOW()"#,
        client_id, req.package_id
    )
    .execute(&state.db)
    .await?;

    // Audit
    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'package_assign', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        caller_id, client_id
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::OK)
}

/// Aggregated stats for the reseller (or admin-level global stats).
async fn reseller_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<ResellerStatsResponse>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let total_clients: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM users
           WHERE deleted_at IS NULL AND role != 'admin'::user_role
             AND ($1::boolean OR reseller_id = $2)"#,
        is_admin, caller_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let active_clients: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM users
           WHERE deleted_at IS NULL AND role != 'admin'::user_role AND status = 'active'
             AND ($1::boolean OR reseller_id = $2)"#,
        is_admin, caller_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let total_sites: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM sites s
           LEFT JOIN users u ON u.id = s.owner_id
           WHERE s.deleted_at IS NULL
             AND ($1::boolean OR u.reseller_id = $2)"#,
        is_admin, caller_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let total_disk_mb: i64 = sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(s.disk_mb), 0)::bigint
           FROM sites s
           LEFT JOIN users u ON u.id = s.owner_id
           WHERE s.deleted_at IS NULL
             AND ($1::boolean OR u.reseller_id = $2)"#,
        is_admin, caller_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let total_disk_used_gb = total_disk_mb as f64 / 1024.0;

    // Revenue: SUM of price_monthly for all assigned packages under this reseller's clients
    let total_revenue_monthly: rust_decimal::Decimal = sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(rp.price_monthly), 0::decimal)
           FROM user_packages up
           JOIN reseller_packages rp ON rp.id = up.package_id
           JOIN users u ON u.id = up.user_id
           WHERE ($1::boolean OR u.reseller_id = $2)"#,
        is_admin, caller_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or_default();

    Ok(Json(ResellerStatsResponse {
        total_clients,
        active_clients,
        total_sites,
        total_disk_used_gb,
        total_revenue_monthly: total_revenue_monthly.to_f64().unwrap_or(0.0),
    }))
}

async fn suspend_client(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let updated = sqlx::query!(
        "UPDATE users SET status = 'suspended'
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR reseller_id = $3)
         RETURNING id",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?;

    if updated.is_none() {
        return Err(ApiError::NotFound { resource: "client" });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn unsuspend_client(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let updated = sqlx::query!(
        "UPDATE users SET status = 'active'
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR reseller_id = $3)
         RETURNING id",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?;

    if updated.is_none() {
        return Err(ApiError::NotFound { resource: "client" });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_client(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let deleted = sqlx::query!(
        "UPDATE users SET deleted_at = NOW()
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR reseller_id = $3)
         RETURNING id",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?;

    if deleted.is_none() {
        return Err(ApiError::NotFound { resource: "client" });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn login_as_client(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<LoginAsResponse>> {
    let (caller_id, is_admin) = require_admin_or_reseller(&claims)?;

    let target = sqlx::query!(
        "SELECT id, email, role::text AS role, wizard_complete FROM users
         WHERE id = $1 AND deleted_at IS NULL AND status = 'active'
           AND ($2::boolean OR reseller_id = $3)",
        client_id, is_admin, caller_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "client" })?;

    let target_role = target.role.unwrap_or_default();
    if target_role == "admin" {
        return Err(ApiError::Forbidden("Cannot impersonate an admin account".into()));
    }

    let exp = Utc::now() + Duration::minutes(60);
    let imp_claims = super::auth::Claims {
        sub:             target.id.to_string(),
        jti:             Uuid::new_v4(),
        email:           target.email.clone(),
        role:            target_role.clone(),
        exp:             exp.timestamp(),
        iat:             Utc::now().timestamp(),
        wizard_complete: target.wizard_complete,
        totp_verified:   true,
        impersonated_by: Some(caller_id.to_string()),
    };

    let token = encode_jwt(&imp_claims, &state.jwt_encoding_key)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("JWT encoding failed: {}", e)))?;

    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'login_as_client', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        caller_id, client_id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(LoginAsResponse {
        access_token:    token,
        user_id:         target.id.to_string(),
        email:           target.email,
        role:            target_role,
        expires_in:      60 * 60, // 1 hour (matches JWT expiry above)
        wizard_complete: target.wizard_complete,
    }))
}

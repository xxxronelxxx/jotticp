use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, Query, State},
    Json,
    http::{StatusCode, HeaderMap},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::{Claims, encode_jwt, hash_password};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    pub search: Option<String>,
    pub role:   Option<String>,
    pub limit:  Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub email:     String,
    pub password:  String,
    pub role:      String,  // "user" | "reseller"
    pub full_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub full_name: Option<String>,
    pub email:     Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangeRoleRequest {
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: Option<String>,
    pub new_password:     String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id:              Uuid,
    pub email:           String,
    pub role:            String,
    pub full_name:       Option<String>,
    pub status:          String,
    pub wizard_complete: bool,
    pub created_at:      DateTime<Utc>,
    pub last_login_at:   Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ImpersonationResponse {
    pub impersonation_token: String,
    pub user_email:          String,
    pub expires_in:          u64,
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit:     Option<i64>,
    pub offset:    Option<i64>,
    pub action:    Option<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to:   Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub user_id:   Option<Uuid>,
    pub action:    Option<String>,
    pub target:    Option<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to:   Option<DateTime<Utc>>,
    pub limit:     Option<i64>,
    pub offset:    Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ActivityEntry {
    pub id:              i64,
    pub action:          String,
    pub resource_type:   Option<String>,
    pub resource_id:     Option<Uuid>,
    pub ip_address:      Option<String>,
    pub created_at:      DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email:      Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────


fn has_html_chars(s: &str) -> bool {
    s.contains('<') || s.contains('>')
}
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/users",                     get(list_users).post(create_user))
        .route("/api/v1/users/me",                   get(get_me))
        .route("/api/v1/users/{id}",                 get(get_user).put(update_user).delete(delete_user))
        .route("/api/v1/users/{id}/impersonate",     post(impersonate_user))
        .route("/api/v1/users/{id}/activity",        get(get_activity))
        .route("/api/v1/users/{id}/role",            put(change_role))
        .route("/api/v1/users/{id}/suspend",         post(suspend_user))
        .route("/api/v1/users/{id}/unsuspend",       post(unsuspend_user))
        .route("/api/v1/users/{id}/password",        put(change_password))
        // Step 4.18 — full audit log (admin only); supports CSV export
        .route("/api/v1/audit-log",                 get(get_audit_log))
}

// ── Access helpers ────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

fn require_admin_or_reseller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let (user_id, is_admin) = caller(claims)?;
    let is_reseller = claims.role == "reseller";
    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Admin or reseller role required".into()));
    }
    Ok((user_id, is_admin))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn get_me(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<UserResponse>> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let r = sqlx::query!(
        "SELECT id, email, role::text AS role, full_name, status, wizard_complete, created_at, last_login_at
         FROM users WHERE id = $1 AND deleted_at IS NULL",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    Ok(Json(UserResponse {
        id:              r.id,
        email:           r.email,
        role:            r.role.unwrap_or_default(),
        full_name:       r.full_name,
        status:          r.status,
        wizard_complete: r.wizard_complete,
        created_at:      r.created_at,
        last_login_at:   r.last_login_at,
    }))
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListUsersQuery>,
) -> ApiResult<Json<Vec<UserResponse>>> {
    let (user_id, is_admin) = require_admin_or_reseller(&claims)?;
    let is_reseller = claims.role == "reseller";
    let limit  = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"SELECT id, email, role::text AS role, full_name, status, wizard_complete, created_at, last_login_at
           FROM users
           WHERE deleted_at IS NULL
             AND ($1::boolean OR reseller_id = $2)
             AND ($3::text IS NULL OR email ILIKE '%' || $3 || '%' OR full_name ILIKE '%' || $3 || '%')
             AND ($4::text IS NULL OR role::text = $4)
           ORDER BY created_at DESC
           LIMIT $5 OFFSET $6"#,
        is_admin, user_id, q.search, q.role, limit, offset
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| UserResponse {
        id:              r.id,
        email:           r.email,
        role:            r.role.unwrap_or_default(),
        full_name:       r.full_name,
        status:          r.status,
        wizard_complete: r.wizard_complete,
        created_at:      r.created_at,
        last_login_at:   r.last_login_at,
    }).collect()))
}

async fn get_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UserResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    let is_reseller = claims.role == "reseller";

    // Users can view their own profile; admins/resellers can view others
    if !is_admin && !is_reseller && user_id != id {
        return Err(ApiError::Forbidden("Cannot view another user's profile".into()));
    }

    let row = sqlx::query!(
        r#"SELECT id, email, role::text AS role, full_name, status, wizard_complete, created_at, last_login_at
           FROM users
           WHERE id = $1 AND deleted_at IS NULL
             AND ($2::boolean OR id = $3 OR reseller_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    Ok(Json(UserResponse {
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

async fn create_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateUserRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
    let (creator_id, is_admin) = require_admin_or_reseller(&claims)?;

    let valid_roles = ["user", "reseller"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err(ApiError::Validation(format!("role must be one of: {}", valid_roles.join(", "))));
    }

    // Only admins can create resellers
    if req.role == "reseller" && !is_admin {
        return Err(ApiError::Forbidden("Only admins can create reseller accounts".into()));
    }

    if req.password.len() < 12 {
        return Err(ApiError::Validation("Password must be at least 12 characters".into()));
    }
    if let Some(ref name) = req.full_name {
        if has_html_chars(name) {
            return Err(ApiError::Validation("full_name contains invalid characters".into()));
        }
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

    let user_id = Uuid::new_v4();
    let reseller_id = if is_admin { None } else { Some(creator_id) };

    sqlx::query!(
        "INSERT INTO users (id, email, password_hash, role, full_name, status,
                            wizard_complete, reseller_id, created_at)
         VALUES ($1, $2, $3, $4::text::user_role, $5, 'active', false, $6, NOW())",
        user_id, email, password_hash, req.role, req.full_name, reseller_id
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(UserResponse {
        id:              user_id,
        email,
        role:            req.role,
        full_name:       req.full_name,
        status:          "active".into(),
        wizard_complete: false,
        created_at:      Utc::now(),
        last_login_at:   None,
    })))
}

async fn update_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> ApiResult<Json<UserResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Users can update themselves; admins can update anyone
    if !is_admin && user_id != id {
        return Err(ApiError::Forbidden("Cannot update another user's profile".into()));
    }

    if let Some(ref name) = req.full_name {
        sqlx::query!("UPDATE users SET full_name = $1 WHERE id = $2", name, id)
            .execute(&state.db).await?;
    }

    if let Some(ref email) = req.email {
        let email = email.to_lowercase().trim().to_string();
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1 AND id != $2 AND deleted_at IS NULL)",
            email, id
        )
        .fetch_one(&state.db)
        .await?
        .unwrap_or(false);

        if exists {
            return Err(ApiError::Validation(format!("Email {} is already in use", email)));
        }
        sqlx::query!("UPDATE users SET email = $1 WHERE id = $2", email, id)
            .execute(&state.db).await?;
    }

    get_user(State(state), claims, Path(id)).await
}

async fn delete_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (_, is_admin) = require_admin_or_reseller(&claims)?;

    if !is_admin {
        return Err(ApiError::Forbidden("Only admins can delete users".into()));
    }

    let target = sqlx::query!(
        "SELECT id, role::text AS role FROM users WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    // Prevent deleting the last admin
    if target.role.as_deref() == Some("admin") {
        let admin_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM users WHERE role::text = 'admin' AND deleted_at IS NULL"
        )
        .fetch_one(&state.db)
        .await?
        .unwrap_or(0);

        if admin_count <= 1 {
            return Err(ApiError::Validation("Cannot delete the last admin account".into()));
        }
    }

    sqlx::query!(
        "UPDATE users SET deleted_at = NOW(), status = 'deleted' WHERE id = $1",
        id
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn impersonate_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(target_id): Path<Uuid>,
) -> ApiResult<Json<ImpersonationResponse>> {
    let (admin_id, is_admin) = caller(&claims)?;
    let is_reseller = claims.role == "reseller";

    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Only admins and resellers can impersonate users".into()));
    }

    // Fetch target user — resellers can only impersonate their own clients
    let target = sqlx::query!(
        "SELECT id, email, role::text AS role, wizard_complete FROM users
         WHERE id = $1 AND deleted_at IS NULL AND status = 'active'
           AND ($2::boolean OR reseller_id = $3)",
        target_id, is_admin, admin_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    // Nobody — including other admins — may impersonate an admin account.
    // Allowing admin-to-admin impersonation would mask accountability in audit logs
    // and create privilege confusion (two simultaneous admin sessions for one identity).
    let target_role = target.role.unwrap_or_default();
    if target_role == "admin" {
        return Err(ApiError::Forbidden("Cannot impersonate an admin account".into()));
    }

    // Build impersonation JWT — 5-min TTL, totp_verified = true so it's usable
    let exp = Utc::now() + Duration::minutes(5);

    // Build impersonation claims — impersonated_by carries the caller's UUID so
    // middleware and audit code can identify impersonated sessions from the token itself.
    let imp_claims = Claims {
        sub:             target.id.to_string(),
        jti:             Uuid::new_v4(),
        email:           target.email.clone(),
        role:            target_role,
        exp:             exp.timestamp(),
        iat:             Utc::now().timestamp(),
        wizard_complete: target.wizard_complete,
        totp_verified:   true,
        impersonated_by: Some(admin_id.to_string()),
    };

    let token = encode_jwt(&imp_claims, &state.jwt_encoding_key)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("JWT encoding failed: {}", e)))?;

    // Audit log
    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'impersonation_start', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        admin_id, target_id
    )
    .execute(&state.db)
    .await?;

    tracing::warn!(
        admin_id = %admin_id,
        target_id = %target_id,
        target_email = %target.email,
        "IMPERSONATION_START"
    );

    Ok(Json(ImpersonationResponse {
        impersonation_token: token,
        user_email:          target.email,
        expires_in:          300,
    }))
}

async fn get_activity(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Query(q): Query<ActivityQuery>,
) -> ApiResult<Json<Vec<ActivityEntry>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let is_reseller = claims.role == "reseller";

    // Users can view own activity; admins/resellers can view others
    if !is_admin && !is_reseller && user_id != id {
        return Err(ApiError::Forbidden("Cannot view another user's activity".into()));
    }

    // Resellers may only view activity of their own clients
    if is_reseller && user_id != id {
        let is_client = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND reseller_id = $2 AND deleted_at IS NULL)",
            id, user_id
        )
        .fetch_one(&state.db)
        .await?
        .unwrap_or(false);

        if !is_client {
            return Err(ApiError::Forbidden("Cannot view activity of users outside your account".into()));
        }
    }

    let limit  = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"SELECT id, action, resource_type, resource_id,
                  ip_address::text AS "ip_address?", created_at
           FROM audit_log
           WHERE user_id = $1
             AND ($2::text IS NULL OR action = $2)
             AND ($3::timestamptz IS NULL OR created_at >= $3)
             AND ($4::timestamptz IS NULL OR created_at <= $4)
           ORDER BY created_at DESC
           LIMIT $5 OFFSET $6"#,
        id, q.action, q.date_from, q.date_to, limit, offset
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ActivityEntry {
        id:              r.id,
        action:          r.action,
        resource_type:   r.resource_type,
        resource_id:     r.resource_id,
        ip_address:      r.ip_address,
        created_at:      r.created_at,
        user_email:      None,
    }).collect()))
}

/// Admin-only full audit log. Supports CSV export via `Accept: text/csv`.
///
/// Filters: user_id, action, target (resource_type), date_from, date_to.
async fn get_audit_log(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<AuditLogQuery>,
    headers: HeaderMap,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;

    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Admin access required for the full audit log".into()));
    }

    let limit  = q.limit.unwrap_or(200).min(1000);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"SELECT al.id, al.user_id, u.email AS "user_email?",
                  al.action, al.resource_type, al.resource_id,
                  al.ip_address::text AS "ip_address?", al.created_at
           FROM audit_log al
           LEFT JOIN users u ON u.id = al.user_id
           WHERE ($1::uuid IS NULL OR al.user_id = $1)
             AND ($2::text IS NULL OR al.action = $2)
             AND ($3::text IS NULL OR al.resource_type = $3)
             AND ($4::timestamptz IS NULL OR al.created_at >= $4)
             AND ($5::timestamptz IS NULL OR al.created_at <= $5)
           ORDER BY al.created_at DESC
           LIMIT $6 OFFSET $7"#,
        q.user_id, q.action, q.target, q.date_from, q.date_to, limit, offset
    )
    .fetch_all(&state.db)
    .await?;

    // CSV export when client sends `Accept: text/csv`
    let wants_csv = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/csv"))
        .unwrap_or(false);

    if wants_csv {
        let mut csv = String::from("id,user_id,user_email,action,resource_type,resource_id,ip_address,created_at\n");
        for r in &rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                r.id,
                r.user_id.map(|id| id.to_string()).unwrap_or_default(),
                r.user_email.as_deref().unwrap_or(""),
                r.action,
                r.resource_type.as_deref().unwrap_or(""),
                r.resource_id.map(|id| id.to_string()).unwrap_or_default(),
                r.ip_address.as_deref().unwrap_or(""),
                r.created_at.to_rfc3339(),
            ));
        }

        return Ok((
            StatusCode::OK,
            [
                ("content-type", "text/csv; charset=utf-8"),
                ("content-disposition", "attachment; filename=\"audit-log.csv\""),
            ],
            csv,
        ).into_response());
    }

    // JSON response
    let entries: Vec<ActivityEntry> = rows.into_iter().map(|r| ActivityEntry {
        id:              r.id,
        action:          r.action,
        resource_type:   r.resource_type,
        resource_id:     r.resource_id,
        ip_address:      r.ip_address,
        created_at:      r.created_at,
        user_email:      r.user_email,
    }).collect();

    Ok(Json(entries).into_response())
}

async fn change_role(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeRoleRequest>,
) -> ApiResult<Json<UserResponse>> {
    let (admin_id, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Only admins can change user roles".into()));
    }

    let valid_roles = ["user", "reseller", "admin"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err(ApiError::Validation(format!("role must be one of: {}", valid_roles.join(", "))));
    }

    // Cannot change own role
    if admin_id == id {
        return Err(ApiError::Validation("Cannot change your own role".into()));
    }

    let affected = sqlx::query!(
        "UPDATE users SET role = $1::text::user_role WHERE id = $2 AND deleted_at IS NULL",
        req.role, id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "user" });
    }

    // Audit log the role change
    sqlx::query!(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, ip_address, success, created_at)
         VALUES ($1, 'role_change', 'user', $2, '127.0.0.1'::inet, true, NOW())",
        admin_id, id
    )
    .execute(&state.db)
    .await?;

    get_user(State(state), claims, Path(id)).await
}

async fn suspend_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (admin_id, is_admin) = require_admin_or_reseller(&claims)?;

    // Resellers can only suspend their own clients
    let _ = sqlx::query!(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL
         AND ($2::boolean OR reseller_id = $3)",
        id, is_admin, admin_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    sqlx::query!(
        "UPDATE users SET status = 'suspended' WHERE id = $1",
        id
    )
    .execute(&state.db)
    .await?;

    // Also suspend all sites owned by this user
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "suspend_user_sites", "user_id": id});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::OK)
}

async fn unsuspend_user(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (admin_id, is_admin) = require_admin_or_reseller(&claims)?;

    let _ = sqlx::query!(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL
         AND ($2::boolean OR reseller_id = $3)",
        id, is_admin, admin_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    sqlx::query!(
        "UPDATE users SET status = 'active' WHERE id = $1",
        id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "unsuspend_user_sites", "user_id": id});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::OK)
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let caller_is_admin = claims.role == "admin";
    let caller_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    if !caller_is_admin && caller_id != id {
        return Err(ApiError::Forbidden("Cannot change another user's password.".into()));
    }

    if req.new_password.len() < 8 {
        return Err(ApiError::Validation("New password must be at least 8 characters.".into()));
    }

    let user = sqlx::query!(
        "SELECT password_hash FROM users WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "user" })?;

    if !caller_is_admin {
        let current = req.current_password.as_deref().unwrap_or("");
        if !super::auth::verify_password(current, &user.password_hash)? {
            return Err(ApiError::Validation("Current password is incorrect.".into()));
        }
    }

    let new_hash = super::auth::hash_password(&req.new_password)?;
    sqlx::query!(
        "UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2",
        new_hash, id
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

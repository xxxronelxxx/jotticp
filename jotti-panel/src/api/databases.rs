use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListDbQuery {
    pub site_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDbRequest {
    pub site_id:  Uuid,
    pub db_type:  String, // "mysql" | "postgres" | "ferretdb" | "valkey" | "surrealdb"
    pub db_name:  String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct DbResponse {
    pub id:         Uuid,
    pub site_id:    Uuid,
    pub db_type:    String,
    pub db_name:    String,
    pub db_user:    String,
    pub status:     String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DbStatsResponse {
    pub id:          Uuid,
    pub db_name:     String,
    pub db_type:     String,
    pub size_bytes:  i64,
    pub table_count: i64,
    pub connection_count: i64,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/databases",                       get(list_databases).post(create_database))
        .route("/api/v1/databases/valkey",                post(create_valkey))
        .route("/api/v1/databases/surrealdb",             post(create_surrealdb))
        .route("/api/v1/databases/{id}",                   get(get_database).delete(delete_database))
        .route("/api/v1/databases/{id}/password",          post(change_password))
        .route("/api/v1/databases/{id}/stats",             get(get_stats))
        .route("/api/v1/databases/{id}/export",            get(db_export))
        .route("/api/v1/databases/{id}/import",            post(db_import))
        .route("/api/v1/databases/{id}/users",             get(db_users_list).post(db_users_create))
        .route("/api/v1/databases/{id}/users/{user_id}",   delete(db_users_delete))
        .route("/api/v1/databases/{id}/phpmyadmin-token",  post(phpmyadmin_token))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn assert_site_owner(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<()> {
    let owned = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3))",
        site_id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !owned {
        return Err(ApiError::NotFound { resource: "site" });
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_databases(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListDbQuery>,
) -> ApiResult<Json<Vec<DbResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let dbs: Vec<DbResponse> = if let Some(site_id) = q.site_id {
        assert_site_owner(&state, site_id, user_id, is_admin).await?;
        sqlx::query!(
            "SELECT ud.id, ud.site_id, ud.db_type, ud.db_name, ud.db_user, ud.status, ud.created_at
             FROM user_databases ud
             WHERE ud.site_id = $1 AND ud.deleted_at IS NULL
             ORDER BY ud.created_at DESC",
            site_id
        )
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|r| DbResponse {
            id:         r.id,
            site_id:    r.site_id,
            db_type:    r.db_type,
            db_name:    r.db_name,
            db_user:    r.db_user,
            status:     r.status,
            created_at: r.created_at,
        })
        .collect()
    } else {
        sqlx::query!(
            r#"SELECT ud.id, ud.site_id, ud.db_type, ud.db_name, ud.db_user, ud.status, ud.created_at
               FROM user_databases ud
               JOIN sites s ON s.id = ud.site_id
               WHERE ud.deleted_at IS NULL AND s.deleted_at IS NULL
                 AND ($1::boolean OR s.owner_id = $2)
               ORDER BY ud.created_at DESC"#,
            is_admin, user_id
        )
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|r| DbResponse {
            id:         r.id,
            site_id:    r.site_id,
            db_type:    r.db_type,
            db_name:    r.db_name,
            db_user:    r.db_user,
            status:     r.status,
            created_at: r.created_at,
        })
        .collect()
    };

    Ok(Json(dbs))
}

async fn create_database(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateDbRequest>,
) -> ApiResult<(StatusCode, Json<DbResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    // Normalize frontend aliases to canonical backend types
    let db_type_normalized = match req.db_type.as_str() {
        "postgresql" => "postgres".to_string(),
        other        => other.to_string(),
    };
    let valid_types = ["mysql", "mariadb", "postgres", "ferretdb"];
    if !valid_types.contains(&db_type_normalized.as_str()) {
        return Err(ApiError::Validation(format!(
            "db_type must be one of: {}. Use /databases/valkey or /databases/surrealdb for those types.",
            valid_types.join(", ")
        )));
    }

    if !is_valid_db_name(&req.db_name) {
        return Err(ApiError::Validation(
            "db_name must be 1-64 chars, alphanumeric and underscores only".into()
        ));
    }

    // Enforce per-site database limit
    let db_count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_databases WHERE site_id = $1 AND db_type = ANY($2) AND deleted_at IS NULL",
        req.site_id,
        &["mysql", "postgres", "ferretdb"] as &[&str]
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if db_count >= state.config.max_databases_per_site as i64 {
        return Err(ApiError::Validation(format!(
            "Site has reached the maximum of {} databases", state.config.max_databases_per_site
        )));
    }

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL",
        req.site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let db_user = format!("orbit_{}_{}", &site.unix_user[..site.unix_user.len().min(16)], &req.db_name[..req.db_name.len().min(8)]);
    let db_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO user_databases (id, site_id, db_type, db_name, db_user, status, created_at)
         VALUES ($1, $2, $3, $4, $5, 'provisioning', NOW())",
        db_id, req.site_id, db_type_normalized, req.db_name, db_user
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_database",
            "db_id": db_id,
            "site_id": req.site_id,
            "db_type": db_type_normalized,
            "db_name": req.db_name,
            "db_user": db_user,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(DbResponse {
        id:         db_id,
        site_id:    req.site_id,
        db_type:    db_type_normalized,
        db_name:    req.db_name,
        db_user,
        status:     "provisioning".into(),
        created_at: Utc::now(),
    })))
}

async fn create_valkey(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateDbRequest>,
) -> ApiResult<(StatusCode, Json<DbResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL",
        req.site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // One Valkey instance per site
    let existing = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_databases WHERE site_id = $1 AND db_type = 'valkey' AND deleted_at IS NULL",
        req.site_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if existing > 0 {
        return Err(ApiError::Validation("Site already has a Valkey instance".into()));
    }

    let db_id = Uuid::new_v4();
    let db_name = format!("valkey_{}", &site.unix_user[..site.unix_user.len().min(16)]);
    let db_user = site.unix_user.clone();

    sqlx::query!(
        "INSERT INTO user_databases (id, site_id, db_type, db_name, db_user, status, created_at)
         VALUES ($1, $2, 'valkey', $3, $4, 'provisioning', NOW())",
        db_id, req.site_id, db_name, db_user
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_valkey",
            "db_id": db_id,
            "site_id": req.site_id,
            "unix_user": site.unix_user,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(DbResponse {
        id:         db_id,
        site_id:    req.site_id,
        db_type:    "valkey".into(),
        db_name,
        db_user,
        status:     "provisioning".into(),
        created_at: Utc::now(),
    })))
}

async fn create_surrealdb(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateDbRequest>,
) -> ApiResult<(StatusCode, Json<DbResponse>)> {
    // SurrealDB is Pro-only
    if !state.config.is_pro() {
        return Err(ApiError::FeatureGated { feature: "surrealdb" });
    }

    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    if !is_valid_db_name(&req.db_name) {
        return Err(ApiError::Validation(
            "db_name must be 1-64 chars, alphanumeric and underscores only".into()
        ));
    }

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL",
        req.site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    let db_id = Uuid::new_v4();
    let db_user = format!("surreal_{}", &site.unix_user[..site.unix_user.len().min(16)]);

    sqlx::query!(
        "INSERT INTO user_databases (id, site_id, db_type, db_name, db_user, status, created_at)
         VALUES ($1, $2, 'surrealdb', $3, $4, 'provisioning', NOW())",
        db_id, req.site_id, req.db_name, db_user
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_surrealdb",
            "db_id": db_id,
            "site_id": req.site_id,
            "db_name": req.db_name,
            "db_user": db_user,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(DbResponse {
        id:         db_id,
        site_id:    req.site_id,
        db_type:    "surrealdb".into(),
        db_name:    req.db_name,
        db_user,
        status:     "provisioning".into(),
        created_at: Utc::now(),
    })))
}

async fn get_database(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DbResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT ud.id, ud.site_id, ud.db_type, ud.db_name, ud.db_user, ud.status, ud.created_at
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1 AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "database" })?;

    Ok(Json(DbResponse {
        id:         row.id,
        site_id:    row.site_id,
        db_type:    row.db_type,
        db_name:    row.db_name,
        db_user:    row.db_user,
        status:     row.status,
        created_at: row.created_at,
    }))
}

async fn delete_database(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let db = sqlx::query!(
        r#"SELECT ud.id, ud.site_id, ud.db_type, ud.db_name, ud.db_user
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1 AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "database" })?;

    sqlx::query!(
        "UPDATE user_databases SET status = 'deleting', deleted_at = NOW() WHERE id = $1",
        id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "deprovision_database",
            "db_id": db.id,
            "site_id": db.site_id,
            "db_type": db.db_type,
            "db_name": db.db_name,
            "db_user": db.db_user,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    if req.password.len() < 12 {
        return Err(ApiError::Validation("Password must be at least 12 characters".into()));
    }
    if !is_strong_password(&req.password) {
        return Err(ApiError::Validation(
            "Password must contain at least one uppercase letter, one lowercase letter, one digit, and one special character".into()
        ));
    }

    let db = sqlx::query!(
        r#"SELECT ud.id, ud.site_id, ud.db_type, ud.db_user
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1 AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "database" })?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "change_db_password",
            "db_id": db.id,
            "site_id": db.site_id,
            "db_type": db.db_type,
            "db_user": db.db_user,
            "new_password": req.password,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

async fn get_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DbStatsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let db = sqlx::query!(
        r#"SELECT ud.id, ud.db_name, ud.db_type, ud.db_user
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1 AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "database" })?;

    // Stats columns not yet populated — return zero values until jotti-agent populates them
    Ok(Json(DbStatsResponse {
        id:               db.id,
        db_name:          db.db_name,
        db_type:          db.db_type,
        size_bytes:       0,
        table_count:      0,
        connection_count: 0,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_db_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && name.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false)
}

/// Require at least one uppercase, one lowercase, one digit, one special char.
fn is_strong_password(pw: &str) -> bool {
    let has_upper   = pw.chars().any(|c| c.is_ascii_uppercase());
    let has_lower   = pw.chars().any(|c| c.is_ascii_lowercase());
    let has_digit   = pw.chars().any(|c| c.is_ascii_digit());
    let has_special = pw.chars().any(|c| !c.is_alphanumeric());
    has_upper && has_lower && has_digit && has_special
}

// ── Database export / import (stubs) ─────────────────────────────────────────

/// GET /api/v1/databases/:id/export
///
/// Export a database as a SQL dump file.
///
/// Ownership: non-admins can only export databases belonging to their own sites.
/// Current implementation: returns 501 — full mysqldump/pg_dump streaming will be
/// added once the jotti-agent gRPC credential-forwarding layer is in place.
/// In the meantime, users can export via phpMyAdmin (accessible from the DB panel).
pub async fn db_export(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    // Verify ownership before returning any error that could reveal DB existence.
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS(
               SELECT 1 FROM user_databases ud
               JOIN sites s ON s.id = ud.site_id
               WHERE ud.id = $1 AND ud.deleted_at IS NULL
                 AND s.deleted_at IS NULL
                 AND ($2::boolean OR s.owner_id = $3)
           )"#,
        id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await;

    match found {
        Ok(Some(true)) => {}
        Ok(_) => return ApiError::NotFound { resource: "database" }.into_response(),
        Err(e) => return ApiError::from(e).into_response(),
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "not_implemented",
            "message": "Database export available in next release. Use phpMyAdmin to export now."
        })),
    )
    .into_response()
}

/// POST /api/v1/databases/:id/import
///
/// Import a SQL dump file into a database.
///
/// Ownership: non-admins can only import into databases belonging to their own sites.
/// Current implementation: returns 501 — multipart streaming import will be added
/// in a future release. In the meantime, users can import via phpMyAdmin.
pub async fn db_import(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    // Verify ownership before returning any error that could reveal DB existence.
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS(
               SELECT 1 FROM user_databases ud
               JOIN sites s ON s.id = ud.site_id
               WHERE ud.id = $1 AND ud.deleted_at IS NULL
                 AND s.deleted_at IS NULL
                 AND ($2::boolean OR s.owner_id = $3)
           )"#,
        id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await;

    match found {
        Ok(Some(true)) => {}
        Ok(_) => return ApiError::NotFound { resource: "database" }.into_response(),
        Err(e) => return ApiError::from(e).into_response(),
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "not_implemented",
            "message": "Database import available in next release. Use phpMyAdmin to import now."
        })),
    )
    .into_response()
}

// ── Database user management (stubs) ─────────────────────────────────────────

async fn db_ownership_check(
    state: &AppState,
    db_id: Uuid,
    user_id: Uuid,
    is_admin: bool,
) -> ApiResult<()> {
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS(
               SELECT 1 FROM user_databases ud
               JOIN sites s ON s.id = ud.site_id
               WHERE ud.id = $1 AND ud.deleted_at IS NULL
                 AND s.deleted_at IS NULL
                 AND ($2::boolean OR s.owner_id = $3)
           )"#,
        db_id, is_admin, user_id
    )
    .fetch_one(&state.db)
    .await?;

    if found != Some(true) {
        return Err(ApiError::NotFound { resource: "database" });
    }
    Ok(())
}

/// GET /api/v1/databases/:id/users
pub async fn db_users_list(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = db_ownership_check(&state, id, user_id, is_admin).await {
        return e.into_response();
    }
    (StatusCode::NOT_IMPLEMENTED, Json(json!({
        "error": "not_implemented",
        "message": "Per-database user management coming in next release.",
        "users": []
    }))).into_response()
}

/// POST /api/v1/databases/:id/users
pub async fn db_users_create(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = db_ownership_check(&state, id, user_id, is_admin).await {
        return e.into_response();
    }
    (StatusCode::NOT_IMPLEMENTED, Json(json!({
        "error": "not_implemented",
        "message": "Per-database user management coming in next release."
    }))).into_response()
}

/// DELETE /api/v1/databases/:id/users/:user_id
pub async fn db_users_delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((id, _user_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = db_ownership_check(&state, id, user_id, is_admin).await {
        return e.into_response();
    }
    (StatusCode::NOT_IMPLEMENTED, Json(json!({
        "error": "not_implemented",
        "message": "Per-database user management coming in next release."
    }))).into_response()
}

/// POST /api/v1/databases/:id/phpmyadmin-token
///
/// Generates a short-lived token and returns a URL that opens phpMyAdmin
/// pre-focused on the requested database.  The token is stored in Valkey
/// with a 5-minute TTL so a future redirect endpoint can validate it without
/// re-checking credentials.
pub async fn phpmyadmin_token(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let (user_id, is_admin) = match caller(&claims) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let db = match sqlx::query!(
        r#"SELECT ud.id, ud.db_name, ud.db_type, ud.db_user, ud.db_password AS "db_password?"
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1 AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None)    => return ApiError::NotFound { resource: "database" }.into_response(),
        Err(e)      => return ApiError::from(e).into_response(),
    };

    // Only MySQL-compatible databases can use the database manager
    if !matches!(db.db_type.as_str(), "mysql" | "mariadb") {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({
            "error": "not_supported",
            "message": "The 1-click login is only available for MySQL/MariaDB databases."
        }))).into_response();
    }

    let token = Uuid::new_v4().to_string();

    // Store token → db info in Valkey with 5-minute TTL
    {
        use redis::AsyncCommands;
        let key = format!("orbit:pma:{}", token);
        let payload = serde_json::json!({
            "db_id":       id,
            "db_name":     db.db_name,
            "db_user":     db.db_user,
            "db_password": db.db_password,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.set_ex(&key, payload.to_string(), 300).await.unwrap_or(());
    }

    // SSO bridge at /phpmyadmin/sso.php handles auto-login
    let url = format!("/phpmyadmin/sso.php?token={}", token);
    (StatusCode::OK, Json(json!({ "token": token, "url": url }))).into_response()
}

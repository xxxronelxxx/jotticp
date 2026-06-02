use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TriggerBackupRequest {
    #[serde(rename = "type")]
    pub backup_type: String, // "full" | "files" | "database"
}

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    pub target:     String,        // "same" | "new_site"
    pub new_domain: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackupSettingsRequest {
    pub daily_time:       Option<String>,    // "03:00"
    pub retention_days:   Option<i32>,
    pub retention_weeks:  Option<i32>,
    pub destination:      Option<String>,    // "local" | "s3" | "sftp"
    pub s3_bucket:        Option<String>,
    pub s3_region:        Option<String>,
    pub s3_access_key:    Option<String>,
    pub s3_secret_key:    Option<String>,
    pub sftp_host:        Option<String>,
    pub sftp_user:        Option<String>,
    pub sftp_path:        Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BackupJobResponse {
    pub id:            Uuid,
    pub site_id:       Uuid,
    #[serde(rename = "type")]
    pub backup_type:   String,
    pub status:        String,
    pub size_bytes:    Option<i64>,
    pub destination:   Option<String>,
    pub started_at:    Option<DateTime<Utc>>,
    pub completed_at:  Option<DateTime<Utc>>,
    pub manifest_path: Option<String>,
    pub created_at:    DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct BackupSettingsResponse {
    pub site_id:         Uuid,
    pub daily_time:      String,
    pub retention_days:  i32,
    pub retention_weeks: i32,
    pub destination:     String,
    pub s3_bucket:       Option<String>,
    pub s3_region:       Option<String>,
    pub sftp_host:       Option<String>,
    pub sftp_user:       Option<String>,
    pub sftp_path:       Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/backups",                          get(list_all_backups))
        .route("/api/v1/backups/settings/{site_id}",      get(get_settings).put(update_settings))
        .route("/api/v1/backups/{site_id}",               get(list_backups).post(trigger_backup))
        .route("/api/v1/backup-jobs/{backup_id}/status",  get(backup_status))
        .route("/api/v1/backup-jobs/{backup_id}/restore", post(restore_backup))
        .route("/api/v1/backup-jobs/{backup_id}",         delete(delete_backup))
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

async fn list_all_backups(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<BackupJobResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT bj.id, bj.site_id, bj.backup_type, bj.status, bj.size_bytes,
                  bj.destination, bj.started_at, bj.completed_at, bj.manifest_path, bj.created_at
           FROM backup_jobs bj
           JOIN sites s ON s.id = bj.site_id
           WHERE s.deleted_at IS NULL AND ($1::boolean OR s.owner_id = $2)
           ORDER BY bj.created_at DESC
           LIMIT 200"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| BackupJobResponse {
        id:            r.id,
        site_id:       r.site_id,
        backup_type:   r.backup_type,
        status:        r.status,
        size_bytes:    r.size_bytes,
        destination:   r.destination,
        started_at:    r.started_at,
        completed_at:  r.completed_at,
        manifest_path: r.manifest_path,
        created_at:    r.created_at,
    }).collect()))
}

async fn list_backups(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<BackupJobResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        "SELECT id, site_id, backup_type, status, size_bytes, destination,
                started_at, completed_at, manifest_path, created_at
         FROM backup_jobs WHERE site_id = $1
         ORDER BY created_at DESC
         LIMIT 100",
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| BackupJobResponse {
        id:            r.id,
        site_id:       r.site_id,
        backup_type:   r.backup_type,
        status:        r.status,
        size_bytes:    r.size_bytes,
        destination:   r.destination,
        started_at:    r.started_at,
        completed_at:  r.completed_at,
        manifest_path: r.manifest_path,
        created_at:    r.created_at,
    }).collect()))
}

async fn trigger_backup(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<TriggerBackupRequest>,
) -> ApiResult<(StatusCode, Json<BackupJobResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let valid_types = ["full", "files", "database"];
    if !valid_types.contains(&req.backup_type.as_str()) {
        return Err(ApiError::Validation(format!(
            "type must be one of: {}", valid_types.join(", ")
        )));
    }

    // Get destination from site backup settings
    let destination: String = sqlx::query_scalar!(
        "SELECT destination FROM backup_settings WHERE site_id = $1",
        site_id
    )
    .fetch_optional(&state.db)
    .await?
    .unwrap_or_else(|| "local".to_string());

    let backup_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO backup_jobs (id, site_id, backup_type, status, destination, created_at)
         VALUES ($1, $2, $3, 'pending', $4, NOW())",
        backup_id, site_id, req.backup_type, destination
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "run_backup",
            "backup_id": backup_id,
            "site_id": site_id,
            "backup_type": req.backup_type,
            "destination": destination,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::ACCEPTED, Json(BackupJobResponse {
        id:            backup_id,
        site_id,
        backup_type:   req.backup_type,
        status:        "pending".into(),
        size_bytes:    None,
        destination:   Some(destination),
        started_at:    None,
        completed_at:  None,
        manifest_path: None,
        created_at:    Utc::now(),
    })))
}

async fn backup_status(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(backup_id): Path<Uuid>,
) -> ApiResult<Json<BackupJobResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT bj.id, bj.site_id, bj.backup_type, bj.status, bj.size_bytes,
                  bj.destination, bj.started_at, bj.completed_at, bj.manifest_path, bj.created_at
           FROM backup_jobs bj
           JOIN sites s ON s.id = bj.site_id
           WHERE bj.id = $1
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        backup_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "backup" })?;

    Ok(Json(BackupJobResponse {
        id:            row.id,
        site_id:       row.site_id,
        backup_type:   row.backup_type,
        status:        row.status,
        size_bytes:    row.size_bytes,
        destination:   row.destination,
        started_at:    row.started_at,
        completed_at:  row.completed_at,
        manifest_path: row.manifest_path,
        created_at:    row.created_at,
    }))
}

async fn restore_backup(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(backup_id): Path<Uuid>,
    Json(req): Json<RestoreRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (user_id, is_admin) = caller(&claims)?;

    let backup = sqlx::query!(
        r#"SELECT bj.id, bj.site_id, bj.status, bj.manifest_path FROM backup_jobs bj
           JOIN sites s ON s.id = bj.site_id
           WHERE bj.id = $1 AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        backup_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "backup" })?;

    if backup.status != "completed" {
        return Err(ApiError::Validation(format!(
            "Cannot restore backup with status '{}'. Only completed backups can be restored.",
            backup.status
        )));
    }

    // Validate restore target against strict allowlist
    let valid_targets = ["same", "new_site"];
    if !valid_targets.contains(&req.target.as_str()) {
        return Err(ApiError::Validation(format!(
            "target must be one of: {}", valid_targets.join(", ")
        )));
    }

    if req.target == "new_site" && req.new_domain.is_none() {
        return Err(ApiError::Validation("new_domain is required when target is 'new_site'".into()));
    }

    // Validate new_domain format if provided
    if let Some(ref domain) = req.new_domain {
        let domain_lower = domain.to_lowercase();
        let domain_re = regex::Regex::new(
            r"^(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$"
        ).unwrap();
        if !domain_re.is_match(&domain_lower) || domain_lower.len() > 253 {
            return Err(ApiError::Validation("new_domain is not a valid domain name".into()));
        }
    }

    let restore_job_id = Uuid::new_v4();

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "restore_backup",
            "restore_job_id": restore_job_id,
            "backup_id": backup.id,
            "site_id": backup.site_id,
            "target": req.target,
            "new_domain": req.new_domain,
            "manifest_path": backup.manifest_path,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({
        "restore_job_id": restore_job_id,
        "backup_id": backup_id,
        "status": "pending",
        "message": "Restore job queued. Site will be unavailable during restore.",
    }))))
}

async fn delete_backup(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(backup_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let backup = sqlx::query!(
        r#"SELECT bj.id, bj.site_id, bj.status, bj.manifest_path FROM backup_jobs bj
           JOIN sites s ON s.id = bj.site_id
           WHERE bj.id = $1 AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        backup_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "backup" })?;

    if backup.status == "running" {
        return Err(ApiError::Validation("Cannot delete a backup that is currently running".into()));
    }

    sqlx::query!("DELETE FROM backup_jobs WHERE id = $1", backup_id)
        .execute(&state.db)
        .await?;

    // Queue filesystem cleanup of backup files
    if let Some(manifest) = backup.manifest_path {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "delete_backup_files",
            "backup_id": backup.id,
            "site_id": backup.site_id,
            "manifest_path": manifest,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Backup settings ───────────────────────────────────────────────────────────

async fn get_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<BackupSettingsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let row = sqlx::query!(
        "SELECT site_id, daily_time, retention_days, retention_weeks, destination,
                s3_bucket, s3_region, sftp_host, sftp_user, sftp_path
         FROM backup_settings WHERE site_id = $1",
        site_id
    )
    .fetch_optional(&state.db)
    .await?;

    // Return defaults if no settings row yet
    let resp = match row {
        Some(r) => BackupSettingsResponse {
            site_id:         r.site_id,
            daily_time:      r.daily_time.format("%H:%M").to_string(),
            retention_days:  r.retention_days,
            retention_weeks: r.retention_weeks,
            destination:     r.destination,
            s3_bucket:       r.s3_bucket,
            s3_region:       r.s3_region,
            sftp_host:       r.sftp_host,
            sftp_user:       r.sftp_user,
            sftp_path:       r.sftp_path,
        },
        None => BackupSettingsResponse {
            site_id,
            daily_time:      "03:00".into(),
            retention_days:  7,
            retention_weeks: 4,
            destination:     "local".into(),
            s3_bucket:       None,
            s3_region:       None,
            sftp_host:       None,
            sftp_user:       None,
            sftp_path:       None,
        },
    };

    Ok(Json(resp))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<BackupSettingsRequest>,
) -> ApiResult<Json<BackupSettingsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    if let Some(ref dest) = req.destination {
        let valid_dests = ["local", "s3", "sftp"];
        if !valid_dests.contains(&dest.as_str()) {
            return Err(ApiError::Validation(format!("destination must be one of: {}", valid_dests.join(", "))));
        }
        if dest == "s3" && req.s3_bucket.is_none() {
            return Err(ApiError::Validation("s3_bucket is required when destination is 's3'".into()));
        }
        if dest == "sftp" && req.sftp_host.is_none() {
            return Err(ApiError::Validation("sftp_host is required when destination is 'sftp'".into()));
        }
    }

    if let Some(ref time) = req.daily_time {
        // Validate HH:MM format
        let re = regex::Regex::new(r"^([01]\d|2[0-3]):([0-5]\d)$").unwrap();
        if !re.is_match(time) {
            return Err(ApiError::Validation("daily_time must be in HH:MM format (24h)".into()));
        }
    }

    if let Some(d) = req.retention_days { if d < 1 || d > 365 {
        return Err(ApiError::Validation("retention_days must be between 1 and 365".into()));
    }}
    if let Some(w) = req.retention_weeks { if w < 1 || w > 52 {
        return Err(ApiError::Validation("retention_weeks must be between 1 and 52".into()));
    }}

    // Parse daily_time from "HH:MM" string to TIME for PostgreSQL
    let daily_time_parsed: Option<chrono::NaiveTime> = req.daily_time
        .as_deref()
        .map(|t| chrono::NaiveTime::parse_from_str(t, "%H:%M"))
        .transpose()
        .map_err(|_| ApiError::Validation("daily_time must be in HH:MM format (24h)".into()))?;

    sqlx::query!(
        "INSERT INTO backup_settings (site_id, daily_time, retention_days, retention_weeks,
                                      destination, s3_bucket, s3_region, s3_access_key, s3_secret_key,
                                      sftp_host, sftp_user, sftp_path)
         VALUES ($1,
                 COALESCE($2, '03:00'::time),
                 COALESCE($3, 7),
                 COALESCE($4, 4),
                 COALESCE($5, 'local'),
                 $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (site_id) DO UPDATE SET
           daily_time      = COALESCE($2, backup_settings.daily_time),
           retention_days  = COALESCE($3, backup_settings.retention_days),
           retention_weeks = COALESCE($4, backup_settings.retention_weeks),
           destination     = COALESCE($5, backup_settings.destination),
           s3_bucket       = COALESCE($6, backup_settings.s3_bucket),
           s3_region       = COALESCE($7, backup_settings.s3_region),
           s3_access_key   = COALESCE($8, backup_settings.s3_access_key),
           s3_secret_key   = COALESCE($9, backup_settings.s3_secret_key),
           sftp_host       = COALESCE($10, backup_settings.sftp_host),
           sftp_user       = COALESCE($11, backup_settings.sftp_user),
           sftp_path       = COALESCE($12, backup_settings.sftp_path)",
        site_id,
        daily_time_parsed as Option<chrono::NaiveTime>,
        req.retention_days,
        req.retention_weeks,
        req.destination,
        req.s3_bucket,
        req.s3_region,
        req.s3_access_key,
        req.s3_secret_key,
        req.sftp_host,
        req.sftp_user,
        req.sftp_path,
    )
    .execute(&state.db)
    .await?;

    get_settings(State(state), claims, Path(site_id)).await
}

use axum::{
    Router,
    routing::{delete, get, post, put},
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
pub struct CreateCronRequest {
    pub site_id:  Uuid,
    /// Cron expression "minute hour day month weekday" (e.g. "0 * * * *")
    pub schedule: Option<String>,
    /// Alternative: individual fields
    pub minute:   Option<String>,
    pub hour:     Option<String>,
    pub day:      Option<String>,
    pub month:    Option<String>,
    pub weekday:  Option<String>,
    pub command:  String,
    pub label:    Option<String>,
    pub enabled:  Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCronRequest {
    pub schedule: Option<String>,
    pub minute:   Option<String>,
    pub hour:     Option<String>,
    pub day:      Option<String>,
    pub month:    Option<String>,
    pub weekday:  Option<String>,
    pub command:  Option<String>,
    pub label:    Option<String>,
    pub enabled:  Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CronJobResponse {
    pub id:             Uuid,
    pub site_id:        Uuid,
    pub site_domain:    Option<String>,
    pub schedule:       String,
    pub minute:         String,
    pub hour:           String,
    pub day:            String,
    pub month:          String,
    pub weekday:        String,
    pub command:        String,
    pub label:          String,
    pub enabled:        bool,
    pub last_run:       Option<DateTime<Utc>>,
    pub last_run_at:    Option<DateTime<Utc>>,
    pub last_exit_code: Option<i32>,
    pub next_run:       Option<String>,
    pub created_at:     DateTime<Utc>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // New flat routes (frontend uses these)
        .route("/api/v1/cron",              get(list_all_cron_jobs).post(create_cron_job_flat))
        .route("/api/v1/cron/{job_id}",      put(update_cron_job_flat).delete(delete_cron_job_flat))
        .route("/api/v1/cron/{job_id}/run",  post(run_cron_job_now))
        // Legacy site-scoped routes (keep for backward compat)
        .route("/api/v1/cron/site/{site_id}",           get(list_cron_jobs))
        .route("/api/v1/cron/site/{site_id}/{job_id}",   put(update_cron_job).delete(delete_cron_job))
        .route("/api/v1/cron/site/{site_id}/{job_id}/log", get(get_cron_log))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn assert_site_owner(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<String> {
    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;
    Ok(site.unix_user)
}

/// Parse "MIN HOUR DAY MON DOW" → (minute, hour, day, month, weekday).
fn parse_schedule(s: &str) -> ApiResult<(String, String, String, String, String)> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(ApiError::Validation(
            format!("schedule must have 5 fields separated by spaces, got: {:?}", s)
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string(), parts[2].to_string(),
        parts[3].to_string(), parts[4].to_string()))
}

fn build_schedule(minute: &str, hour: &str, day: &str, month: &str, weekday: &str) -> String {
    format!("{} {} {} {} {}", minute, hour, day, month, weekday)
}

fn make_response(
    id: Uuid, site_id: Uuid, site_domain: Option<String>,
    minute: String, hour: String, day: String, month: String, weekday: String,
    command: String, label: Option<String>, enabled: bool,
    last_run_at: Option<DateTime<Utc>>, last_exit_code: Option<i32>,
    created_at: DateTime<Utc>,
) -> CronJobResponse {
    let schedule = build_schedule(&minute, &hour, &day, &month, &weekday);
    CronJobResponse {
        id, site_id, site_domain,
        schedule,
        minute, hour, day, month, weekday,
        command, label: label.unwrap_or_default(), enabled,
        last_run: last_run_at, last_run_at,
        last_exit_code, next_run: None,
        created_at,
    }
}

// ── New flat handlers ─────────────────────────────────────────────────────────

async fn list_all_cron_jobs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<CronJobResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT cj.id, cj.site_id AS "site_id!", s.domain AS site_domain,
                  cj.minute, cj.hour, cj.day, cj.month, cj.weekday,
                  cj.command, cj.label, cj.enabled, cj.last_run_at,
                  NULL::int4 AS last_exit_code, cj.created_at
           FROM cron_jobs cj
           JOIN sites s ON s.id = cj.site_id
           WHERE cj.deleted_at IS NULL AND s.deleted_at IS NULL
             AND ($1::boolean OR s.owner_id = $2)
           ORDER BY cj.created_at ASC"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| make_response(
        r.id, r.site_id, Some(r.site_domain),
        r.minute, r.hour, r.day, r.month, r.weekday,
        r.command, r.label, r.enabled, r.last_run_at, r.last_exit_code,
        r.created_at,
    )).collect()))
}

async fn create_cron_job_flat(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateCronRequest>,
) -> ApiResult<(StatusCode, Json<CronJobResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    // Parse schedule from combined field or individual fields
    let (minute, hour, day, month, weekday) = if let Some(ref sched) = req.schedule {
        parse_schedule(sched)?
    } else {
        (
            req.minute.clone().unwrap_or_else(|| "*".into()),
            req.hour.clone().unwrap_or_else(|| "*".into()),
            req.day.clone().unwrap_or_else(|| "*".into()),
            req.month.clone().unwrap_or_else(|| "*".into()),
            req.weekday.clone().unwrap_or_else(|| "*".into()),
        )
    };

    validate_cron_field(&minute,  0, 59, "minute")?;
    validate_cron_field(&hour,    0, 23, "hour")?;
    validate_cron_field(&day,     1, 31, "day")?;
    validate_cron_field(&month,   1, 12, "month")?;
    validate_cron_field(&weekday, 0,  7, "weekday")?;
    validate_cron_command(&req.command)?;

    let job_count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM cron_jobs WHERE site_id = $1 AND deleted_at IS NULL",
        req.site_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    if job_count >= state.config.max_cron_jobs_per_site as i64 {
        return Err(ApiError::Validation(format!(
            "Site has reached the maximum of {} cron jobs", state.config.max_cron_jobs_per_site
        )));
    }

    let job_id = Uuid::new_v4();
    let label = req.label.clone().unwrap_or_default();
    let enabled = req.enabled.unwrap_or(true);
    let schedule_str = build_schedule(&minute, &hour, &day, &month, &weekday);

    sqlx::query!(
        "INSERT INTO cron_jobs (id, site_id, owner_id, unix_user, schedule, minute, hour, day, month, weekday, command, label, enabled, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NOW())",
        job_id, req.site_id, user_id, unix_user, schedule_str, minute, hour, day, month, weekday, req.command, label, enabled
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_cron", "site_id": req.site_id,
            "unix_user": unix_user, "action": "create",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    let site_domain = sqlx::query_scalar!(
        "SELECT domain FROM sites WHERE id = $1", req.site_id
    ).fetch_optional(&state.db).await?;

    Ok((StatusCode::CREATED, Json(make_response(
        job_id, req.site_id, site_domain,
        minute, hour, day, month, weekday,
        req.command, Some(label), enabled, None, None, Utc::now(),
    ))))
}

async fn update_cron_job_flat(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(job_id): Path<Uuid>,
    Json(req): Json<UpdateCronRequest>,
) -> ApiResult<Json<CronJobResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Find the job and verify ownership
    let row = sqlx::query!(
        r#"SELECT cj.id, cj.site_id AS "site_id!", s.domain AS site_domain, s.unix_user,
                  cj.minute, cj.hour, cj.day, cj.month, cj.weekday, cj.command, cj.label,
                  cj.enabled, cj.last_run_at, cj.created_at
           FROM cron_jobs cj
           JOIN sites s ON s.id = cj.site_id
           WHERE cj.id = $1 AND cj.deleted_at IS NULL AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        job_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "cron_job" })?;

    // Resolve schedule fields
    let (new_minute, new_hour, new_day, new_month, new_weekday) =
        if let Some(ref sched) = req.schedule {
            parse_schedule(sched)?
        } else {
            (
                req.minute.clone().unwrap_or_else(|| row.minute.clone()),
                req.hour.clone().unwrap_or_else(|| row.hour.clone()),
                req.day.clone().unwrap_or_else(|| row.day.clone()),
                req.month.clone().unwrap_or_else(|| row.month.clone()),
                req.weekday.clone().unwrap_or_else(|| row.weekday.clone()),
            )
        };

    if req.schedule.is_some() || req.minute.is_some() { validate_cron_field(&new_minute,  0, 59, "minute")?; }
    if req.schedule.is_some() || req.hour.is_some()   { validate_cron_field(&new_hour,    0, 23, "hour")?; }
    if req.schedule.is_some() || req.day.is_some()    { validate_cron_field(&new_day,     1, 31, "day")?; }
    if req.schedule.is_some() || req.month.is_some()  { validate_cron_field(&new_month,   1, 12, "month")?; }
    if req.schedule.is_some() || req.weekday.is_some(){ validate_cron_field(&new_weekday, 0,  7, "weekday")?; }
    if let Some(ref cmd) = req.command { validate_cron_command(cmd)?; }

    let new_command = req.command.unwrap_or(row.command);
    let new_label   = req.label.unwrap_or_else(|| row.label.unwrap_or_default());
    let new_enabled = req.enabled.unwrap_or(row.enabled);

    sqlx::query!(
        "UPDATE cron_jobs SET minute=$1, hour=$2, day=$3, month=$4, weekday=$5,
         command=$6, label=$7, enabled=$8 WHERE id=$9",
        new_minute, new_hour, new_day, new_month, new_weekday,
        new_command, new_label, new_enabled, job_id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_cron", "site_id": row.site_id,
            "unix_user": row.unix_user, "action": "update",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(Json(make_response(
        row.id, row.site_id, Some(row.site_domain),
        new_minute, new_hour, new_day, new_month, new_weekday,
        new_command, Some(new_label), new_enabled,
        row.last_run_at, None, row.created_at,
    )))
}

async fn delete_cron_job_flat(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(job_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT cj.site_id AS "site_id!", s.unix_user
           FROM cron_jobs cj JOIN sites s ON s.id = cj.site_id
           WHERE cj.id = $1 AND cj.deleted_at IS NULL AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        job_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "cron_job" })?;

    sqlx::query!(
        "UPDATE cron_jobs SET deleted_at = NOW(), enabled = false WHERE id = $1",
        job_id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_cron", "site_id": row.site_id,
            "unix_user": row.unix_user, "action": "delete", "job_id": job_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn run_cron_job_now(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(job_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT cj.id, cj.site_id AS "site_id!", s.unix_user, cj.command
           FROM cron_jobs cj JOIN sites s ON s.id = cj.site_id
           WHERE cj.id = $1 AND cj.deleted_at IS NULL AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        job_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "cron_job" })?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "run_cron_now",
            "job_id": row.id,
            "site_id": row.site_id,
            "unix_user": row.unix_user,
            "command": row.command,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

// ── Legacy site-scoped handlers ───────────────────────────────────────────────

async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<CronJobResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        r#"SELECT id, site_id AS "site_id!", minute, hour, day, month, weekday, command, label,
                enabled, last_run_at, NULL::int4 AS last_exit_code, created_at
         FROM cron_jobs WHERE site_id = $1 AND deleted_at IS NULL
         ORDER BY created_at ASC"#,
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| make_response(
        r.id, r.site_id, None,
        r.minute, r.hour, r.day, r.month, r.weekday,
        r.command, r.label, r.enabled, r.last_run_at, r.last_exit_code,
        r.created_at,
    )).collect()))
}

async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, job_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateCronRequest>,
) -> ApiResult<Json<CronJobResponse>> {
    // Delegate to flat handler but verify site ownership first
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let row = sqlx::query!(
        "SELECT id FROM cron_jobs WHERE id = $1 AND site_id = $2 AND deleted_at IS NULL",
        job_id, site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "cron_job" })?;

    // Re-use flat handler
    update_cron_job_flat(
        State(state),
        claims,
        Path(row.id),
        Json(req),
    ).await
}

async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, job_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;
    delete_cron_job_flat(State(state), claims, Path(job_id)).await
}

async fn get_cron_log(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, job_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let unix_user = assert_site_owner(&state, site_id, user_id, is_admin).await?;
    let _ = sqlx::query!(
        "SELECT id FROM cron_jobs WHERE id = $1 AND site_id = $2 AND deleted_at IS NULL",
        job_id, site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "cron_job" })?;

    let log_path = format!("/var/log/orbit/cron/{}/{}.log", unix_user, job_id);
    let content = read_last_lines(&log_path, 100);
    Ok(Json(serde_json::json!({ "job_id": job_id, "site_id": site_id, "log": content })))
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn validate_cron_field(value: &str, min: i32, max: i32, field: &str) -> ApiResult<()> {
    if value == "*" { return Ok(()); }
    if let Some(rest) = value.strip_prefix("*/") {
        let n: i32 = rest.parse().map_err(|_| ApiError::Validation(format!("{}: invalid step '{}'", field, value)))?;
        if n < 1 { return Err(ApiError::Validation(format!("{}: step must be >= 1", field))); }
        return Ok(());
    }
    for part in value.split(',') {
        if let Some((lo, hi)) = part.split_once('-') {
            let lo: i32 = lo.parse().map_err(|_| ApiError::Validation(format!("{}: invalid range '{}'", field, part)))?;
            let hi: i32 = hi.parse().map_err(|_| ApiError::Validation(format!("{}: invalid range '{}'", field, part)))?;
            if lo < min || hi > max || lo > hi {
                return Err(ApiError::Validation(format!("{}: range {}-{} out of bounds ({}-{})", field, lo, hi, min, max)));
            }
        } else {
            let n: i32 = part.parse().map_err(|_| ApiError::Validation(format!("{}: invalid value '{}'", field, part)))?;
            if n < min || n > max {
                return Err(ApiError::Validation(format!("{}: {} out of range ({}-{})", field, n, min, max)));
            }
        }
    }
    Ok(())
}

fn validate_cron_command(cmd: &str) -> ApiResult<()> {
    if cmd.is_empty() {
        return Err(ApiError::Validation("Cron command cannot be empty".into()));
    }
    if cmd.len() > 1024 {
        return Err(ApiError::Validation("Cron command exceeds 1024 characters".into()));
    }
    Ok(())
}

fn read_last_lines(path: &str, n: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Step 5.9 — Staging Environments (Pro only)
///
/// Endpoints under /api/v1/sites/{id}/staging/*
///
/// All operations are gated behind the "staging" Pro feature flag.
/// File sync uses rsync; DB clone uses mysqldump|mysql or pg_dump|psql.
use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json,
};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Uuid;
use tracing::{info, instrument};

use crate::{AppState, ApiError, ApiResult};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Validate that a unix username is safe to embed in shell commands delegated
/// to the agent.  Only alphanumeric, underscore, and hyphen; max 32 chars; must
/// not start with a hyphen (useradd restriction).  Prevents shell injection
/// through site.unix_user values that might have been set before this check
/// was introduced.
fn validate_unix_user(u: &str) -> ApiResult<()> {
    let re = Regex::new(r"^[a-zA-Z0-9_][a-zA-Z0-9_-]{0,31}$").unwrap();
    if !re.is_match(u) {
        return Err(ApiError::Validation(format!(
            "Invalid unix username '{u}': must start with letter/digit/underscore,              contain only [a-zA-Z0-9_-], and be at most 32 characters"
        )));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// DB row type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow, Serialize)]
struct StagingSiteRow {
    id:                 Uuid,
    production_site_id: Uuid,
    staging_domain:     String,
    unix_user:          String,
    status:             String,
    created_at:         chrono::DateTime<Utc>,
    last_synced_at:     Option<chrono::DateTime<Utc>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// DTOs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateStagingRequest {
    /// Override the auto-generated staging unix user (optional).
    staging_unix_user: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Enforce a Pro feature gate.  Returns 402 if the feature is not enabled
/// for the current user's plan.
async fn require_feature(
    state: &AppState,
    user_id: Uuid,
    feature: &str,
) -> ApiResult<()> {
    // Look up whether the user's assigned package has the requested feature flag.
    // Schema: user_packages → reseller_packages.feature_flags (JSONB)
    // feature_flags keys: "staging", "dns_editing", "php_version_switch", etc.
    let feature_flags: Option<serde_json::Value> = sqlx::query_scalar(
        r#"
        SELECT rp.feature_flags
        FROM   user_packages    up
        JOIN   reseller_packages rp ON rp.id = up.package_id
        WHERE  up.user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("feature gate query failed: {e}")))?;

    let enabled = feature_flags
        .as_ref()
        .and_then(|flags| flags.get(feature))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if enabled {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::PAYMENT_REQUIRED,
            format!("feature '{feature}' requires a Pro plan upgrade"),
        ))
    }
}

/// Run a shell command via orbit-agent HTTP endpoint.
/// For file operations we delegate to the agent; rsync/mysqldump/etc are
/// wrapped in JSON commands the agent executes as root.
async fn run_shell_cmd(
    state: &AppState,
    site_id: Uuid,
    cmd: &str,
) -> ApiResult<String> {
    let agent_url = format!(
        "{}/internal/sites/{}/exec",
        state.config.agent_base_url, site_id
    );
    let resp = state
        .http
        .post(&agent_url)
        .json(&json!({ "cmd": cmd }))
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("agent request failed: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError::internal(format!("agent exec failed: {body}")));
    }

    let result: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::internal(format!("agent response parse failed: {e}")))?;

    Ok(result["stdout"].as_str().unwrap_or("").to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/sites/{id}/staging/create
///
/// Clones the production site to a staging environment:
/// 1. Pro feature check
/// 2. rsync files from production home → staging home (exclude .git)
/// 3. Clone DB: mysqldump SOURCE_DB | mysql STAGING_DB
/// 4. Create staging vhost: staging.DOMAIN
/// 5. Insert staging_sites row
#[instrument(skip(state))]
async fn create_staging(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    require_feature(&state, caller_id, "staging").await?;

    // Load production site
    let site = sqlx::query!(
        r#"SELECT id, domain, unix_user, owner_id FROM sites WHERE id = $1 AND owner_id = $2"#,
        site_id,
        caller_id,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "site not found"))?;

    // Check no staging already exists
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM staging_sites WHERE production_site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("staging query failed: {e}")))?;

    if existing.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a staging environment already exists for this site; delete it first",
        ));
    }

    // Validate unix_user from DB before embedding in shell commands.
    // Although site.unix_user was set by OrbitCP at provisioning time, we validate
    // it defensively here because the staging commands are forwarded to the agent.
    validate_unix_user(&site.unix_user)?;

    let staging_unix_user = format!("stg_{}", site.unix_user);
    let staging_domain = format!("staging.{}", site.domain);
    let prod_home = format!("/home/{}/", site.unix_user);
    let staging_home = format!("/home/{}/", staging_unix_user);
    // staging_unix_user inherits the validation of site.unix_user — stg_ prefix is safe

    // 1. Create staging unix user + homedir (via agent)
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "useradd -m -d /home/{u} -s /usr/sbin/nologin {u} 2>/dev/null || true",
            u = staging_unix_user
        ),
    )
    .await?;

    // 2. rsync files (exclude .git)
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "rsync -a --exclude '.git' {src} {dst}",
            src = prod_home,
            dst = staging_home,
        ),
    )
    .await?;

    // 3. Clone DB (MySQL example; PG path would be pg_dump | psql)
    //    We look up the site's primary database for cloning.
    let db_row = sqlx::query!(
        r#"SELECT db_name, db_type::TEXT AS "db_type!" FROM databases WHERE site_id = $1 ORDER BY created_at ASC LIMIT 1"#,
        site_id,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("db query failed: {e}")))?;

    if let Some(db) = db_row {
        let staging_db_name = format!("stg_{}", db.db_name);
        let clone_cmd = if db.db_type == "postgresql" {
            format!(
                "createdb {dst} 2>/dev/null || true && pg_dump {src} | psql {dst}",
                src = db.db_name,
                dst = staging_db_name,
            )
        } else {
            format!(
                "mysql -e 'CREATE DATABASE IF NOT EXISTS `{dst}`' && \
                 mysqldump {src} | mysql {dst}",
                src = db.db_name,
                dst = staging_db_name,
            )
        };
        run_shell_cmd(&state, site_id, &clone_cmd).await?;
    }

    // 4. Create staging vhost (delegate to agent)
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "orbit-agent vhost create --domain {domain} --user {user} --webroot /home/{user}/public_html",
            domain = staging_domain,
            user = staging_unix_user,
        ),
    )
    .await?;

    // 5. Insert staging_sites row
    let staging_id = Uuid::new_v4();
    sqlx::query!(
        r#"
        INSERT INTO staging_sites (id, production_site_id, staging_domain, unix_user, status)
        VALUES ($1, $2, $3, $4, 'ready')
        "#,
        staging_id,
        site_id,
        staging_domain,
        staging_unix_user,
    )
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("insert staging_sites failed: {e}")))?;

    info!(site_id = %site_id, staging_id = %staging_id, domain = %staging_domain, "staging environment created");

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "staging_site_id": staging_id,
            "staging_url": format!("https://{}", staging_domain),
        })),
    ))
}

/// GET /api/v1/sites/{id}/staging
///
/// Returns the staging site row or null if none exists.
#[instrument(skip(state))]
async fn get_staging(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    // Verify site ownership
    let _site = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sites WHERE id = $1 AND owner_id = $2",
    )
    .bind(site_id)
    .bind(caller_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "site not found"))?;

    let row = sqlx::query_as::<_, StagingSiteRow>(
        "SELECT * FROM staging_sites WHERE production_site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("staging query failed: {e}")))?;

    match row {
        Some(s) => Ok(Json(json!({
            "id":                 s.id,
            "production_site_id": s.production_site_id,
            "staging_domain":     s.staging_domain,
            "staging_url":        format!("https://{}", s.staging_domain),
            "unix_user":          s.unix_user,
            "status":             s.status,
            "created_at":         s.created_at,
            "last_synced_at":     s.last_synced_at,
        }))),
        None => Ok(Json(json!(null))),
    }
}

/// POST /api/v1/sites/{id}/staging/sync
///
/// Resyncs files from production to staging (rsync --delete).
/// Returns synced_at and files_changed count.
#[instrument(skip(state))]
async fn sync_staging(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    require_feature(&state, caller_id, "staging").await?;

    let staging = get_staging_row(&state, site_id, caller_id).await?;

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1",
        site_id,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?;

    let prod_home = format!("/home/{}/", site.unix_user);
    let staging_home = format!("/home/{}/", staging.unix_user);

    // rsync with --delete and --itemize-changes to count changed files
    let output = run_shell_cmd(
        &state,
        site_id,
        &format!(
            "rsync -a --delete --itemize-changes {src} {dst}",
            src = prod_home,
            dst = staging_home,
        ),
    )
    .await?;

    let files_changed = output.lines().filter(|l| !l.trim().is_empty()).count();
    let synced_at = Utc::now();

    sqlx::query!(
        "UPDATE staging_sites SET last_synced_at = $1 WHERE id = $2",
        synced_at,
        staging.id,
    )
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("update staging failed: {e}")))?;

    info!(site_id = %site_id, files_changed, "staging synced from production");

    Ok(Json(json!({
        "synced_at":     synced_at,
        "files_changed": files_changed,
    })))
}

/// POST /api/v1/sites/{id}/staging/push-to-live
///
/// Copies staging files to production (rsync staging → production).
/// Returns pushed_at and files_pushed count.
#[instrument(skip(state))]
async fn push_to_live(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    require_feature(&state, caller_id, "staging").await?;

    let staging = get_staging_row(&state, site_id, caller_id).await?;

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1",
        site_id,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?;

    let prod_home = format!("/home/{}/", site.unix_user);
    let staging_home = format!("/home/{}/", staging.unix_user);

    // rsync staging → production, capture itemize output to count changes
    let output = run_shell_cmd(
        &state,
        site_id,
        &format!(
            "rsync -a --delete --itemize-changes {src} {dst}",
            src = staging_home,
            dst = prod_home,
        ),
    )
    .await?;

    let files_pushed = output.lines().filter(|l| !l.trim().is_empty()).count();
    let pushed_at = Utc::now();

    info!(site_id = %site_id, files_pushed, "staging pushed to live");

    Ok(Json(json!({
        "pushed_at":    pushed_at,
        "files_pushed": files_pushed,
    })))
}

/// GET /api/v1/sites/{id}/staging/diff
///
/// Returns a file diff between staging and production.
/// Uses rsync --dry-run --itemize-changes to identify changed files.
#[instrument(skip(state))]
async fn staging_diff(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    require_feature(&state, caller_id, "staging").await?;

    let staging = get_staging_row(&state, site_id, caller_id).await?;

    let site = sqlx::query!(
        "SELECT unix_user FROM sites WHERE id = $1",
        site_id,
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?;

    let prod_home = format!("/home/{}/", site.unix_user);
    let staging_home = format!("/home/{}/", staging.unix_user);

    // Dry-run rsync to get the diff
    let output = run_shell_cmd(
        &state,
        site_id,
        &format!(
            "rsync -a --dry-run --delete --itemize-changes {src} {dst}",
            src = staging_home,
            dst = prod_home,
        ),
    )
    .await?;

    // Parse rsync itemize output.
    // Format: <YXcstpoguax> path
    // Y = update type: '>' sent, '<' received, 'c' change/creation, '*' message
    // X = file type: 'f' file, 'd' directory, 'L' symlink
    let changed_files: Vec<Value> = output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            if line.len() < 12 {
                return None;
            }
            let flags = &line[..11];
            let path = line[12..].trim().to_string();
            if path.is_empty() {
                return None;
            }
            let status = if flags.contains('*') || flags.contains('d') {
                "deleted"
            } else if flags.chars().next() == Some('>') || flags.chars().next() == Some('<') {
                "modified"
            } else {
                "added"
            };
            // Rough size diff — rsync doesn't give us bytes in itemize; use 0 as placeholder
            Some(json!({
                "path":           path,
                "status":         status,
                "size_diff_bytes": 0,
            }))
        })
        .collect();

    info!(site_id = %site_id, changed = changed_files.len(), "staging diff computed");

    Ok(Json(json!({ "changed_files": changed_files })))
}

/// DELETE /api/v1/sites/{id}/staging
///
/// Tears down the staging environment:
/// - Remove staging homedir
/// - Drop staging DB
/// - Remove vhost
/// - Delete systemd units
/// - Delete from staging_sites table
#[instrument(skip(state))]
async fn delete_staging(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    require_feature(&state, caller_id, "staging").await?;

    let staging = get_staging_row(&state, site_id, caller_id).await?;

    // Remove homedir
    run_shell_cmd(
        &state,
        site_id,
        &format!("userdel -rf {} 2>/dev/null || true", staging.unix_user),
    )
    .await?;

    // Remove vhost
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "orbit-agent vhost delete --domain {} 2>/dev/null || true",
            staging.staging_domain
        ),
    )
    .await?;

    // Drop staging DB (best-effort)
    let db_name = format!("stg_{}", staging.unix_user);
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "mysql -e 'DROP DATABASE IF EXISTS `{db}`' 2>/dev/null; \
             dropdb --if-exists {db} 2>/dev/null; true",
            db = db_name,
        ),
    )
    .await?;

    // Remove systemd units
    run_shell_cmd(
        &state,
        site_id,
        &format!(
            "systemctl stop php-fpm-{u}.service 2>/dev/null; \
             systemctl disable php-fpm-{u}.service 2>/dev/null; \
             rm -f /etc/systemd/system/php-fpm-{u}.service; \
             systemctl daemon-reload 2>/dev/null; true",
            u = staging.unix_user,
        ),
    )
    .await?;

    // Delete from DB
    sqlx::query!("DELETE FROM staging_sites WHERE id = $1", staging.id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("delete staging_sites failed: {e}")))?;

    info!(site_id = %site_id, staging_id = %staging.id, "staging environment deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the staging row for a production site, verifying the caller owns the site.
async fn get_staging_row(
    state: &AppState,
    site_id: Uuid,
    caller_id: Uuid,
) -> ApiResult<StagingSiteRow> {
    // Verify ownership
    let _owned = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sites WHERE id = $1 AND owner_id = $2",
    )
    .bind(site_id)
    .bind(caller_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("site query failed: {e}")))?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "site not found"))?;

    sqlx::query_as::<_, StagingSiteRow>(
        "SELECT * FROM staging_sites WHERE production_site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("staging query failed: {e}")))?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no staging environment exists for this site"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/{id}/staging/create",
            post(create_staging),
        )
        .route(
            "/{id}/staging",
            get(get_staging).delete(delete_staging),
        )
        .route("/{id}/staging/sync",        post(sync_staging))
        .route("/{id}/staging/push-to-live", post(push_to_live))
        .route("/{id}/staging/diff",        get(staging_diff))
}

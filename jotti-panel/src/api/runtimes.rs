//! Runtime management API — direct provisioning (no agent queue)
//!
//! GET  /api/v1/sites/{id}/runtime
//! POST /api/v1/sites/{id}/runtime
//! GET  /api/v1/sites/{id}/nodejs/status
//! POST /api/v1/sites/{id}/nodejs/process
//! GET  /api/v1/sites/{id}/nodejs/logs
//! GET  /api/v1/sites/{id}/python/logs

use axum::{
    Router,
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Constants ─────────────────────────────────────────────────────────────────

const VALID_RUNTIMES: &[&str] = &["php", "nodejs", "python"];

const DEFAULT_VERSIONS: &[(&str, &str)] = &[
    ("nodejs", "20"),
    ("python", "3.12"),
];

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetRuntimeRequest {
    pub runtime:     String,
    pub version:     Option<String>,
    /// Python entry point, e.g. "main:app", "wsgi:application".
    /// Ignored for nodejs/php.
    pub entry_point: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeResponse {
    pub site_id:        Uuid,
    pub runtime:        String,
    pub version:        String,
    pub runtime_status: String,
    pub runtime_error:  Option<String>,
    pub port:           Option<i32>,
    pub entry_point:    Option<String>,
    pub message:        String,
}

#[derive(Debug, Serialize)]
pub struct NodeProcessStatus {
    pub name:      String,
    pub status:    String,
    pub pid:       Option<i64>,
    pub uptime:    Option<i64>,
    pub restarts:  i64,
    pub cpu:       f64,
    pub memory_mb: f64,
}

#[derive(Debug, Deserialize)]
pub struct NodeProcessAction {
    pub action: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/sites/{site_id}/runtime",
               axum::routing::get(get_runtime).post(set_runtime))
        .route("/api/v1/sites/{site_id}/nodejs/status",
               axum::routing::get(nodejs_status))
        .route("/api/v1/sites/{site_id}/nodejs/process",
               axum::routing::post(nodejs_process))
        .route("/api/v1/sites/{site_id}/nodejs/logs",
               axum::routing::get(nodejs_logs))
        .route("/api/v1/sites/{site_id}/python/logs",
               axum::routing::get(python_logs))
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    Ok((user_id, claims.role == "admin"))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/sites/{site_id}/runtime
async fn get_runtime(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<RuntimeResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT id,
                  runtime          AS "runtime?",
                  runtime_version  AS "runtime_version?",
                  runtime_status,
                  runtime_error    AS "runtime_error?",
                  runtime_entry    AS "runtime_entry?",
                  service_port     AS "service_port?"
           FROM sites
           WHERE id = $1 AND deleted_at IS NULL
             AND ($2::boolean OR owner_id = $3)"#,
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    Ok(Json(RuntimeResponse {
        site_id,
        runtime:        row.runtime.unwrap_or_else(|| "php".into()),
        version:        row.runtime_version.unwrap_or_default(),
        runtime_status: row.runtime_status,
        runtime_error:  row.runtime_error,
        port:           row.service_port,
        entry_point:    row.runtime_entry,
        message:        String::new(),
    }))
}

/// POST /api/v1/sites/{site_id}/runtime
///
/// Provisions the requested runtime directly (no agent queue).
/// Returns 202 immediately; polls GET /runtime for status.
async fn set_runtime(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<SetRuntimeRequest>,
) -> ApiResult<(StatusCode, Json<RuntimeResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;

    // Validate runtime
    let runtime = req.runtime.to_lowercase();
    if !VALID_RUNTIMES.contains(&runtime.as_str()) {
        return Err(ApiError::Validation(format!(
            "runtime must be one of: {}", VALID_RUNTIMES.join(", ")
        )));
    }

    // Resolve version
    let version = req.version
        .as_deref()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            DEFAULT_VERSIONS.iter()
                .find(|(r, _)| *r == runtime)
                .map(|(_, v)| *v)
                .unwrap_or("latest")
        })
        .to_string();

    if runtime != "php" && (!version.chars().all(|c| c.is_ascii_digit() || c == '.') || version.len() > 8) {
        return Err(ApiError::Validation(format!(
            "version must be digits and dots only (e.g. \"3.12\", \"20\"); got {:?}", version
        )));
    }

    // Entry point for Python — validate early to return 422 instead of async failure
    let entry_point = req.entry_point
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("main:app")
        .to_string();

    if runtime == "python" {
        let ep_re = regex::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_.]*:[a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
        if !ep_re.is_match(&entry_point) {
            return Err(ApiError::Validation(format!(
                "entry_point must be 'module:callable' (e.g. 'main:app'); got: {:?}", entry_point
            )));
        }
    }

    // Fetch site
    let site = sqlx::query!(
        "SELECT id, domain, unix_user, web_server, php_version,
                runtime AS \"prev_runtime?\",
                service_port AS \"service_port?\"
         FROM sites
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // PHP: just clear the runtime columns, nothing to provision
    if runtime == "php" {
        let prev = site.prev_runtime.as_deref().unwrap_or("php");
        if prev != "php" {
            let dom = site.domain.clone();
            let usr = site.unix_user.clone();
            let prv = prev.to_string();
            // Use actual DB values for php_version and web_server when restoring PHP vhost
            let php_ver = site.php_version.clone();
            let web_sv  = site.web_server.clone();
            tokio::task::spawn_blocking(move || {
                let _ = crate::services::provisioning::deprovision_runtime_sync(&dom, &usr, &prv);
                // Restore the correct vhost type (nginx/apache/ols) with actual php_version
                let job = crate::services::provisioning::ProvisioningJob {
                    site_id: uuid::Uuid::nil(),
                    domain:      dom.clone(),
                    php_version: php_ver,
                    web_server:  web_sv,
                    unix_user:   usr.clone(),
                    server_id:   None,
                };
                let _ = crate::services::provisioning::write_php_vhost_pub(&job);
                let _ = std::process::Command::new("nginx").args(["-t"]).output();
                let _ = std::process::Command::new("systemctl").args(["reload", "nginx"]).output();
            });
        }
        sqlx::query!(
            "UPDATE sites SET runtime = 'php', runtime_version = NULL,
             runtime_status = 'idle', runtime_error = NULL,
             runtime_entry = NULL, service_port = NULL, updated_at = NOW()
             WHERE id = $1",
            site_id
        ).execute(&state.db).await?;

        return Ok((StatusCode::OK, Json(RuntimeResponse {
            site_id,
            runtime: "php".into(),
            version: String::new(),
            runtime_status: "idle".into(),
            runtime_error: None,
            port: None,
            entry_point: None,
            message: "Switched back to PHP.".into(),
        })));
    }

    // Atomically allocate port (or reuse existing) + mark provisioning in one DB round-trip.
    // The CTE avoids the TOCTOU race: two concurrent requests cannot pick the same port.
    let port: i32 = sqlx::query_scalar!(
        r#"WITH free AS (
            SELECT s FROM generate_series(3001, 4999) s
            WHERE s NOT IN (SELECT service_port FROM sites WHERE service_port IS NOT NULL AND id != $1)
            ORDER BY s LIMIT 1
        )
        UPDATE sites
        SET runtime = $2, runtime_version = $3, runtime_entry = $4,
            service_port = COALESCE(service_port, (SELECT s FROM free)),
            runtime_status = 'provisioning', runtime_error = NULL, updated_at = NOW()
        WHERE id = $1
        RETURNING service_port"#,
        site_id, runtime, version, entry_point
    )
    .fetch_one(&state.db)
    .await?
    .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("No free runtime port available (3001-4999)")))?;

    // Spawn background provisioning — releases HTTP response immediately
    let state2      = Arc::clone(&state);
    let domain      = site.domain.clone();
    let unix_user   = site.unix_user.clone();
    let prev_rt     = site.prev_runtime.clone().unwrap_or_else(|| "php".into());
    let rt          = runtime.clone();
    let ep          = entry_point.clone();

    tokio::spawn(async move {
        let dom2 = domain.clone();
        let usr2 = unix_user.clone();
        let rt2  = rt.clone();
        let ep2  = ep.clone();
        let prev = prev_rt.clone();
        let p    = port;

        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            // 1. Tear down previous runtime
            if prev != "php" && prev != rt2 {
                // Switching from a different non-PHP runtime — full deprovision
                crate::services::provisioning::deprovision_runtime_sync(&dom2, &usr2, &prev)?;
            } else if prev == rt2 {
                // Re-provisioning the same runtime — only remove nginx vhost (will be re-written)
                // Keep the systemd unit running until the new one replaces it via enable --now
                let available = format!("/etc/nginx/sites-available/{}.conf", dom2);
                let enabled   = format!("/etc/nginx/sites-enabled/{}.conf",   dom2);
                let _ = std::fs::remove_file(&enabled);
                let _ = std::fs::remove_file(&available);
            }
            // prev == "php": no runtime vhost to remove (nginx vhost is PHP type, overwritten below)

            // 2. Provision new runtime
            match rt2.as_str() {
                "nodejs" => crate::services::provisioning::provision_nodejs_sync(&dom2, &usr2, p),
                "python" => crate::services::provisioning::provision_python_sync(&dom2, &usr2, p, &ep2),
                other    => anyhow::bail!("unsupported runtime: {}", other),
            }
        }).await;

        let (status, error_msg): (&str, Option<String>) = match result {
            Ok(Ok(())) => {
                tracing::info!(site_id = %site_id, runtime = %rt, port = p, "Runtime provisioned OK");
                ("active", None)
            }
            Ok(Err(e)) => {
                tracing::error!(site_id = %site_id, runtime = %rt, error = %e, "Runtime provisioning failed");
                ("error", Some(e.to_string()))
            }
            Err(e) => {
                tracing::error!(site_id = %site_id, "Runtime provisioning task panicked: {:?}", e);
                ("error", Some("Internal provisioning error".into()))
            }
        };

        let _ = sqlx::query!(
            "UPDATE sites SET runtime_status = $1, runtime_error = $2, updated_at = NOW()
             WHERE id = $3",
            status, error_msg, site_id
        ).execute(&state2.db).await;
    });

    tracing::info!(site_id = %site_id, domain = %site.domain, %runtime, port, "Runtime provisioning started");

    Ok((StatusCode::ACCEPTED, Json(RuntimeResponse {
        site_id,
        runtime,
        version,
        runtime_status: "provisioning".into(),
        runtime_error:  None,
        port:           Some(port),
        entry_point:    Some(entry_point),
        message:        "Provisioning started. Poll GET /runtime for status.".into(),
    })))
}

// ── Node.js process management (systemd) ─────────────────────────────────────

async fn resolve_site(
    state:    &AppState,
    site_id:  Uuid,
    user_id:  Uuid,
    is_admin: bool,
) -> ApiResult<(String, String, Option<i32>)> {
    let row = sqlx::query!(
        "SELECT unix_user, domain, service_port AS \"service_port?\"
         FROM sites
         WHERE id = $1 AND deleted_at IS NULL
           AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;

    // Re-validate domain from DB before using in filesystem paths or commands
    crate::services::provisioning::validate_domain_pub(&row.domain)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    crate::services::provisioning::validate_unix_user_pub(&row.unix_user)
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    Ok((row.unix_user, row.domain, row.service_port))
}

/// GET /api/v1/sites/{site_id}/nodejs/status — reads from systemd
async fn nodejs_status(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<NodeProcessStatus>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain, _) = resolve_site(&state, site_id, user_id, is_admin).await?;

    let unit = format!("jotticp-nodejs-{}", domain);

    let out = tokio::process::Command::new("systemctl")
        .args(["show", &unit,
               "--property=ActiveState,SubState,MainPID,ExecMainStartTimestamp,NRestarts"])
        .output().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("systemctl show: {}", e)))?;

    let props: std::collections::HashMap<String, String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, '=');
            let k = parts.next()?.trim().to_string();
            let v = parts.next()?.trim().to_string();
            Some((k, v))
        })
        .collect();

    let active_state = props.get("ActiveState").map(|s| s.as_str()).unwrap_or("unknown");
    let sub_state    = props.get("SubState").map(|s| s.as_str()).unwrap_or("unknown");
    let pid: Option<i64> = props.get("MainPID")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&p| p > 0);
    let restarts: i64 = props.get("NRestarts")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let status = match active_state {
        "active"   => if sub_state == "running" { "online" } else { sub_state },
        "failed"   => "errored",
        "inactive" => "stopped",
        other      => other,
    };

    Ok(Json(NodeProcessStatus {
        name:      domain,
        status:    status.to_string(),
        pid,
        uptime:    None,
        restarts,
        cpu:       0.0,
        memory_mb: 0.0,
    }))
}

/// POST /api/v1/sites/{site_id}/nodejs/process — maps to systemctl actions
async fn nodejs_process(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<NodeProcessAction>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain, _) = resolve_site(&state, site_id, user_id, is_admin).await?;

    let unit = format!("jotticp-nodejs-{}", domain);

    let systemctl_action = match req.action.as_str() {
        "start"   => "start",
        "stop"    => "stop",
        "restart" => "restart",
        _ => return Err(ApiError::Validation("action must be: start, stop, restart".into())),
    };

    let out = tokio::process::Command::new("systemctl")
        .args([systemctl_action, &unit])
        .output().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("systemctl: {}", e)))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::ExternalService(format!("systemctl {} {}: {}", systemctl_action, unit, stderr)));
    }

    Ok(Json(serde_json::json!({
        "action":  req.action,
        "domain":  domain,
        "success": true,
        "output":  String::from_utf8_lossy(&out.stdout).trim(),
    })))
}

/// GET /api/v1/sites/{site_id}/nodejs/logs — reads from log files
async fn nodejs_logs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain, _) = resolve_site(&state, site_id, user_id, is_admin).await?;

    let log_dir = format!("/var/log/jotticp/sites/{}", domain);
    let out_log = format!("{}/nodejs.log",       log_dir);
    let err_log = format!("{}/nodejs-error.log", log_dir);

    let stdout = tokio::process::Command::new("tail")
        .args(["-n", "100", &out_log]).output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let stderr = tokio::process::Command::new("tail")
        .args(["-n", "100", &err_log]).output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "stdout": stdout, "stderr": stderr,
        "out_log_path": out_log, "err_log_path": err_log,
    })))
}

/// GET /api/v1/sites/{site_id}/python/logs
async fn python_logs(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain, _) = resolve_site(&state, site_id, user_id, is_admin).await?;

    let log_dir = format!("/var/log/jotticp/sites/{}", domain);
    let out_log = format!("{}/python.log",       log_dir);
    let err_log = format!("{}/python-error.log", log_dir);

    let stdout = tokio::process::Command::new("tail")
        .args(["-n", "100", &out_log]).output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let stderr = tokio::process::Command::new("tail")
        .args(["-n", "100", &err_log]).output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "stdout": stdout, "stderr": stderr,
        "out_log_path": out_log, "err_log_path": err_log,
    })))
}

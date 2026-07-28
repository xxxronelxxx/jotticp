use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState, ApiError, ApiResult};
use crate::services::performance as perf_svc;
use super::auth::Claims;

// ── Hardening ─────────────────────────────────────────────────────────────────

const HARDENING_CONF: &str = "/etc/sysctl.d/99-jottiecp-hardening.conf";

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct PanelSettings {
    pub panel_domain:       String,
    pub admin_email:        String,
    pub smtp_from:          String,
    pub orbit_env:          String,  // "community" | "pro" | "enterprise"
    pub max_upload_mb:      u64,
    pub acme_directory:     String,
    pub default_web_server: String,  // "ols" | "nginx" | "apache"
    pub default_php:        String,
    pub language:           String,
    pub timezone:           String,
    pub date_format:        String,
    pub maintenance_mode:   bool,
    pub registration_open:  bool,
    pub version:            String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub panel_domain:       Option<String>,
    pub admin_email:        Option<String>,
    pub smtp_from:          Option<String>,
    pub max_upload_mb:      Option<u64>,
    pub default_web_server: Option<String>,
    pub maintenance_mode:   Option<bool>,
    pub registration_open:  Option<bool>,
    pub default_php:        Option<String>,
    pub language:           Option<String>,
    pub timezone:           Option<String>,
    pub date_format:        Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SmtpTestResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct LicenseInfo {
    pub plan:       String,
    pub expires_at: Option<String>,
    pub servers:    i32,
    pub max_servers: i32,
}

#[derive(Debug, Serialize)]
pub struct HardeningStatus {
    /// True if /etc/sysctl.d/99-jottiecp-hardening.conf exists.
    pub applied:     bool,
    /// Key=value pairs parsed from the hardening config.
    pub parameters:  std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct PerformanceReport {
    pub http3_supported: bool,
    pub ttfb_ms:         f64,
    pub active_sites:    u64,
    pub nginx_version:   String,
    pub cache_hit_rate:  f64,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct EmailRateLimits {
    pub per_hour:   i32,
    pub per_day:    i32,
    pub per_month:  i32,
    pub enabled:    bool,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/settings",                    get(get_settings).put(update_settings))
        .route("/api/v1/settings/license",            get(get_license))
        .route("/api/v1/settings/smtp/test",          axum::routing::post(test_smtp))
        .route("/api/v1/settings/hardening-status",   get(get_hardening_status))
        .route("/api/v1/settings/performance",        get(get_performance))
        .route("/api/v1/settings/wizard-complete",    post(wizard_complete))
        .route("/api/v1/settings/email-limits",       get(get_email_limits).put(update_email_limits))
        .route("/api/v1/host/stats",                  get(get_host_stats))
        .route("/api/v1/host/services",               get(get_services))
        .route("/api/v1/host/services/{name}/restart", post(restart_service))
        .route("/api/v1/host/top-sites",              get(get_top_sites))
}

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn get_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<PanelSettings>> {
    require_admin(&claims)?;

    // Most settings come from runtime config; mutable ones from DB key-value store
    let maintenance = get_db_setting(&state, "maintenance_mode").await
        .map(|v| v == "true")
        .unwrap_or(false);
    let registration = get_db_setting(&state, "registration_open").await
        .map(|v| v == "true")
        .unwrap_or(true);
    let default_ws = get_db_setting(&state, "default_web_server").await
        .unwrap_or_else(|| "ols".into());
    let language = get_db_setting(&state, "language").await
        .unwrap_or_else(|| "ru".into());
    let timezone = get_db_setting(&state, "timezone").await
        .unwrap_or_else(|| "UTC".into());
    let date_format = get_db_setting(&state, "date_format").await
        .unwrap_or_else(|| "DD/MM/YYYY".into());
    let default_php = get_db_setting(&state, "default_php").await
        .unwrap_or_else(|| "8.3".into());
    let panel_domain = get_db_setting(&state, "panel_domain").await
        .unwrap_or_else(|| state.config.panel_domain.clone());

    Ok(Json(PanelSettings {
        panel_domain,
        admin_email:        state.config.admin_email.clone(),
        smtp_from:          state.config.smtp_from.clone(),
        orbit_env:          state.config.orbit_env.to_string(),
        max_upload_mb:      state.config.max_upload_mb,
        acme_directory:     state.config.acme_directory.clone(),
        default_web_server: default_ws,
        default_php,
        language,
        timezone,
        date_format,
        maintenance_mode:   maintenance,
        registration_open:  registration,
        version:            env!("CARGO_PKG_VERSION").into(),
    }))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<UpdateSettingsRequest>,
) -> ApiResult<Json<PanelSettings>> {
    require_admin(&claims)?;

    if let Some(ref email) = req.admin_email {
        if !email.contains('@') {
            return Err(ApiError::Validation("Invalid admin email".into()));
        }
        set_db_setting(&state, "admin_email", email).await?;
    }

    if let Some(ref smtp_from) = req.smtp_from {
        if !smtp_from.contains('@') {
            return Err(ApiError::Validation("Invalid smtp_from email".into()));
        }
        set_db_setting(&state, "smtp_from", smtp_from).await?;
    }

    if let Some(mb) = req.max_upload_mb {
        if mb < 1 || mb > 10240 {
            return Err(ApiError::Validation("max_upload_mb must be 1–10240".into()));
        }
        set_db_setting(&state, "max_upload_mb", &mb.to_string()).await?;
    }

    if let Some(ref ws) = req.default_web_server {
        if !matches!(ws.as_str(), "ols" | "nginx" | "apache" | "lse") {
            return Err(ApiError::Validation("default_web_server must be ols, nginx, apache, or lse".into()));
        }
        set_db_setting(&state, "default_web_server", ws).await?;
    }

    if let Some(maint) = req.maintenance_mode {
        set_db_setting(&state, "maintenance_mode", if maint { "true" } else { "false" }).await?;
    }

    if let Some(reg) = req.registration_open {
        set_db_setting(&state, "registration_open", if reg { "true" } else { "false" }).await?;
    }

    if let Some(ref domain) = req.panel_domain {
        set_db_setting(&state, "panel_domain", domain).await?;
    }

    if let Some(ref lang) = req.language {
        if !matches!(lang.as_str(), "en" | "ru" | "ar" | "fr" | "es" | "de" | "pt" | "zh-CN") {
            return Err(ApiError::Validation("Invalid language".into()));
        }
        set_db_setting(&state, "language", lang).await?;
    }

    if let Some(ref tz) = req.timezone {
        set_db_setting(&state, "timezone", tz).await?;
    }

    if let Some(ref df) = req.date_format {
        set_db_setting(&state, "date_format", df).await?;
    }

    if let Some(ref php) = req.default_php {
        set_db_setting(&state, "default_php", php).await?;
    }

    tracing::info!(admin = %claims.sub, "panel settings updated");
    get_settings(State(state), claims).await
}

async fn get_license(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<LicenseInfo>> {
    require_admin(&claims)?;

    let server_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM servers WHERE deleted_at IS NULL AND status != 'deleted'"
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    let (plan, max_servers) = match state.config.orbit_env {
        crate::config::OrbitEnv::Community  => ("community", 3),
        crate::config::OrbitEnv::Pro        => ("pro", 999),
        crate::config::OrbitEnv::Enterprise => ("enterprise", 9999),
    };

    Ok(Json(LicenseInfo {
        plan:        plan.into(),
        expires_at:  None, // TODO: license server integration
        servers:     server_count as i32,
        max_servers,
    }))
}

async fn test_smtp(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<SmtpTestResponse>> {
    require_admin(&claims)?;

    // TODO: integrate actual SMTP test in services/email.rs
    // For now, return a placeholder indicating the feature is wired up
    Ok(Json(SmtpTestResponse {
        success: false,
        message: "SMTP test not yet configured — set SMTP_HOST in environment".into(),
    }))
}

// ── Hardening status ──────────────────────────────────────────────────────────

/// GET /api/v1/settings/hardening-status
/// Returns whether CIS Level 1 hardening has been applied and lists the active parameters.
async fn get_hardening_status(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<HardeningStatus>> {
    require_admin(&claims)?;

    let path = std::path::Path::new(HARDENING_CONF);
    let applied = path.exists();

    let mut parameters = std::collections::HashMap::new();
    if applied {
        if let Ok(content) = tokio::fs::read_to_string(HARDENING_CONF).await {
            for line in content.lines() {
                let line = line.trim();
                // Skip comments and blank lines
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                // Parse key=value
                if let Some((key, value)) = line.split_once('=') {
                    parameters.insert(
                        key.trim().to_string(),
                        value.trim().to_string(),
                    );
                }
            }
        }
    }

    Ok(Json(HardeningStatus { applied, parameters }))
}

// ── Performance report ────────────────────────────────────────────────────────

/// GET /api/v1/settings/performance
/// Returns HTTP/3 support status, estimated TTFB, active site count,
/// nginx version, and actionable performance recommendations.
/// Admin-only.
async fn get_performance(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<PerformanceReport>> {
    require_admin(&claims)?;

    let report = perf_svc::get_performance_report(&state).await;

    Ok(Json(PerformanceReport {
        http3_supported: report.http3_supported,
        ttfb_ms:         report.ttfb_ms,
        active_sites:    report.active_sites,
        nginx_version:   report.nginx_version,
        cache_hit_rate:  report.cache_hit_rate,
        recommendations: report.recommendations,
    }))
}

// ── DB key-value helpers ──────────────────────────────────────────────────────

async fn get_db_setting(state: &AppState, key: &str) -> Option<String> {
    sqlx::query_scalar!(
        "SELECT value FROM panel_settings WHERE key = $1",
        key
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

async fn wizard_complete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<StatusCode> {
    let user_id: uuid::Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    sqlx::query!(
        "UPDATE users SET wizard_complete = true, updated_at = NOW() WHERE id = $1",
        user_id
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_db_setting(state: &AppState, key: &str, value: &str) -> ApiResult<()> {
    sqlx::query!(
        "INSERT INTO panel_settings (key, value, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()",
        key, value
    )
    .execute(&state.db)
    .await?;
    Ok(())
}

// ── Email rate limit handlers ─────────────────────────────────────────────────

async fn get_email_limits(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<EmailRateLimits>> {
    require_admin(&claims)?;

    let limits = match get_db_setting(&state, "email_rate_limits").await {
        Some(json_str) => serde_json::from_str::<EmailRateLimits>(&json_str)
            .unwrap_or_default(),
        None => EmailRateLimits::default(),
    };

    Ok(Json(limits))
}

async fn update_email_limits(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<EmailRateLimits>,
) -> ApiResult<Json<EmailRateLimits>> {
    require_admin(&claims)?;

    let json_str = serde_json::to_string(&req)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Serialization failed: {}", e)))?;
    set_db_setting(&state, "email_rate_limits", &json_str).await?;

    Ok(Json(req))
}

// ── Host Stats ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HostStats {
    pub cpu_pct:       f64,
    pub ram_total_mb:  u64,
    pub ram_used_mb:   u64,
    pub ram_free_mb:   u64,
    pub disk_total_gb: f64,
    pub disk_used_gb:  f64,
    pub disk_free_gb:  f64,
    pub disk_pct:      f64,
    pub load_1m:       f64,
    pub load_5m:       f64,
    pub load_15m:      f64,
    pub uptime_secs:   u64,
    pub process_count: u32,
}

/// GET /api/v1/host/stats — real CPU/RAM/disk/load for the panel host server.
async fn get_host_stats(
    claims: Claims,
) -> ApiResult<Json<HostStats>> {
    require_admin(&claims)?;
    Ok(Json(sample_host_metrics().await))
}

/// Sample the local host's live CPU/RAM/disk/load/uptime directly from /proc + df.
/// Reused by the servers API so the local (panel) host reports metrics without an agent.
pub async fn sample_host_metrics() -> HostStats {
    // Two /proc/stat samples 250ms apart for current CPU%
    let (t1, i1) = parse_proc_stat();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let (t2, i2) = parse_proc_stat();

    let dt = t2.saturating_sub(t1);
    let di = i2.saturating_sub(i1);
    let cpu_pct = if dt == 0 { 0.0 } else {
        let busy = dt.saturating_sub(di);
        ((busy as f64 / dt as f64) * 1000.0).round() / 10.0  // one decimal
    };

    // RAM from /proc/meminfo
    let (ram_total_mb, ram_used_mb, ram_free_mb) = read_meminfo();

    // Disk via `df /`
    let (disk_total_gb, disk_used_gb, disk_free_gb) = read_disk_gb();
    let disk_pct = if disk_total_gb > 0.0 {
        ((disk_used_gb / disk_total_gb) * 1000.0).round() / 10.0
    } else { 0.0 };

    // Load from /proc/loadavg
    let (load_1m, load_5m, load_15m) = read_loadavg();

    // Uptime from /proc/uptime
    let uptime_secs = read_uptime_secs();

    // Process count from /proc/loadavg field 4 (running/total)
    let process_count = read_process_count();

    HostStats {
        cpu_pct, ram_total_mb, ram_used_mb, ram_free_mb,
        disk_total_gb, disk_used_gb, disk_free_gb, disk_pct,
        load_1m, load_5m, load_15m, uptime_secs, process_count,
    }
}

fn parse_proc_stat() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let line = content.lines().next().unwrap_or("");
    let nums: Vec<u64> = line.split_whitespace()
        .skip(1).take(8)
        .filter_map(|s| s.parse().ok())
        .collect();
    let idle = nums.get(3).copied().unwrap_or(0)
             + nums.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = nums.iter().sum();
    (total, idle)
}

fn read_meminfo() -> (u64, u64, u64) {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total_kb = 0u64;
    let mut free_kb  = 0u64;
    let mut avail_kb = 0u64;
    for line in content.lines() {
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let val: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:"     => total_kb = val,
            "MemFree:"      => free_kb  = val,
            "MemAvailable:" => avail_kb = val,
            _ => {}
        }
    }
    let total_mb = total_kb / 1024;
    let avail_mb = avail_kb / 1024;
    let used_mb  = total_mb.saturating_sub(avail_mb);
    let free_mb  = free_kb / 1024;
    (total_mb, used_mb, free_mb)
}

fn read_disk_gb() -> (f64, f64, f64) {
    // df -B 1 / — get bytes
    let out = std::process::Command::new("df")
        .args(["-B", "1", "/"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });
    let text = String::from_utf8_lossy(&out.stdout);
    if let Some(line) = text.lines().nth(1) {
        let parts: Vec<u64> = line.split_whitespace()
            .skip(1).take(3)
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() == 3 {
            let gb = 1_073_741_824.0_f64;
            let total = parts[0] as f64 / gb;
            let used  = parts[1] as f64 / gb;
            let free  = parts[2] as f64 / gb;
            return (
                (total * 10.0).round() / 10.0,
                (used  * 10.0).round() / 10.0,
                (free  * 10.0).round() / 10.0,
            );
        }
    }
    (0.0, 0.0, 0.0)
}

fn read_loadavg() -> (f64, f64, f64) {
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut it = content.split_whitespace();
    let a: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let b: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let c: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    (a, b, c)
}

fn read_uptime_secs() -> u64 {
    let content = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    content.split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0) as u64
}

fn read_process_count() -> u32 {
    // /proc/loadavg 4th field is "running/total"
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let field = content.split_whitespace().nth(3).unwrap_or("0/0");
    field.split('/').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

// ── Services ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub name:    String,
    pub label:   String,
    pub active:  bool,
    pub enabled: bool,
    pub pid:     Option<u32>,
}

const MANAGED_SERVICES: &[(&str, &str)] = &[
    ("nginx",        "Nginx"),
    ("php8.3-fpm",   "PHP-FPM"),
    ("postgresql",   "PostgreSQL"),
    ("mariadb",      "MariaDB"),
    ("valkey",       "Valkey (Cache)"),
    ("cloudflared",  "Cloudflare Tunnel"),
    ("fail2ban",     "Fail2ban"),
    ("jotti-panel",  "Jotti Panel"),
    ("orbit-ui",     "Orbit UI"),
];

async fn get_services(claims: Claims) -> ApiResult<Json<Vec<ServiceStatus>>> {
    require_admin(&claims)?;

    let mut handles = Vec::new();
    for (name, label) in MANAGED_SERVICES {
        let n = name.to_string();
        let l = label.to_string();
        handles.push(tokio::spawn(async move {
            let active = tokio::process::Command::new("systemctl")
                .args(["is-active", "--quiet", &n])
                .status().await.map(|s| s.success()).unwrap_or(false);
            let enabled = tokio::process::Command::new("systemctl")
                .args(["is-enabled", "--quiet", &n])
                .status().await.map(|s| s.success()).unwrap_or(false);
            let pid_out = tokio::process::Command::new("systemctl")
                .args(["show", &n, "--property=MainPID", "--value"])
                .output().await.ok();
            let pid: Option<u32> = pid_out
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u32>().ok())
                .filter(|&p| p > 0);
            ServiceStatus { name: n, label: l, active, enabled, pid }
        }));
    }

    let mut services = Vec::new();
    for h in handles {
        if let Ok(s) = h.await {
            services.push(s);
        }
    }
    Ok(Json(services))
}

async fn restart_service(
    claims: Claims,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    let allowed: Vec<&str> = MANAGED_SERVICES.iter().map(|(n, _)| *n).collect();
    if !allowed.contains(&name.as_str()) {
        return Err(ApiError::Validation(format!(
            "Service '{}' is not in the managed list", name
        )));
    }

    // Spawn restart in background so the response is sent before jotti-panel itself may restart.
    let svc = name.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let _ = tokio::process::Command::new("systemctl")
            .args(["restart", &svc])
            .output().await;
    });

    Ok(StatusCode::OK)
}

// ── Top Sites ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TopSite {
    pub id:         String,
    pub domain:     String,
    pub unix_user:  String,
    pub web_server: String,
    pub status:     String,
    pub disk_mb:    u64,
    pub req_count:  u64,
}

async fn get_top_sites(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<TopSite>>> {
    require_admin(&claims)?;

    let rows = sqlx::query!(
        r#"SELECT id, domain, unix_user, web_server, status::text AS "status!"
           FROM sites WHERE deleted_at IS NULL
           ORDER BY created_at DESC LIMIT 50"#
    )
    .fetch_all(&state.db)
    .await?;

    // Collect static row data first, then spawn concurrent disk/log tasks
    struct SiteRow { id: String, domain: String, unix_user: String, web_server: String, status: String }
    let site_rows: Vec<SiteRow> = rows.into_iter().map(|r| SiteRow {
        id:         r.id.to_string(),
        domain:     r.domain,
        unix_user:  r.unix_user,
        web_server: r.web_server,
        status:     r.status,
    }).collect();

    let mut handles: Vec<tokio::task::JoinHandle<(u64, u64)>> = Vec::new();
    for row in &site_rows {
        let user = row.unix_user.clone();
        let log_path = format!("/home/{}/logs/access.log", user);
        handles.push(tokio::spawn(async move {
            let disk_mb = tokio::process::Command::new("du")
                .args(["-sm", &format!("/home/{}", user)])
                .output().await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()))
                .unwrap_or(0);

            let req_count = tokio::process::Command::new("wc")
                .args(["-l", &log_path])
                .output().await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()))
                .unwrap_or(0);

            (disk_mb, req_count)
        }));
    }

    let mut top: Vec<TopSite> = Vec::new();
    for (row, h) in site_rows.into_iter().zip(handles.into_iter()) {
        let (disk_mb, req_count) = h.await.unwrap_or((0, 0));
        top.push(TopSite {
            id:         row.id,
            domain:     row.domain,
            unix_user:  row.unix_user,
            web_server: row.web_server,
            status:     row.status,
            disk_mb,
            req_count,
        });
    }

    top.sort_by(|a, b| b.disk_mb.cmp(&a.disk_mb));
    top.truncate(10);

    Ok(Json(top))
}

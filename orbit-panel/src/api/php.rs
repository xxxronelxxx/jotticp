use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Allowed INI settings with their defaults ─────────────────────────────────

const PHP_SETTINGS_DEFAULTS: &[(&str, &str)] = &[
    ("memory_limit",                "128M"),
    ("max_execution_time",          "30"),
    ("max_input_time",              "60"),
    ("upload_max_filesize",         "2M"),
    ("post_max_size",               "8M"),
    ("max_file_uploads",            "20"),
    ("max_input_vars",              "1000"),
    ("display_errors",              "Off"),
    ("log_errors",                  "On"),
    ("error_reporting",             "production"),
    ("session.gc_maxlifetime",      "1440"),
    ("session.save_handler",        "files"),
    ("session.cookie_httponly",     "1"),
    ("session.use_strict_mode",     "1"),
    ("date.timezone",               "UTC"),
    ("opcache.enable",              "1"),
    ("opcache.memory_consumption",  "128"),
    ("opcache.jit",                 "off"),
    ("opcache.validate_timestamps", "1"),
    ("opcache.revalidate_freq",     "2"),
];

// ── Extension catalogue ───────────────────────────────────────────────────────

const AVAILABLE_EXTENSIONS: &[(&str, bool)] = &[
    // Core/Required (always on)
    ("json",        true),
    ("mbstring",    true),
    ("xml",         true),
    ("dom",         true),
    ("simplexml",   true),
    ("tokenizer",   true),
    ("ctype",       true),
    ("fileinfo",    true),
    // Databases
    ("pdo_mysql",   true),
    ("pdo_pgsql",   true),
    ("pdo_sqlite",  false),
    ("mysqli",      true),
    ("redis",       true),
    ("mongodb",     false),
    ("memcached",   false),
    // Networking
    ("curl",        true),
    ("soap",        true),
    ("sockets",     false),
    ("openssl",     true),
    // Image processing
    ("gd",          true),
    ("imagick",     true),
    ("exif",        false),
    // Math/encoding
    ("bcmath",      true),
    ("gmp",         false),
    ("sodium",      true),
    ("hash",        true),
    // Compression
    ("zip",         true),
    ("zlib",        true),
    ("bz2",         false),
    // Internationalisation
    ("intl",        true),
    ("iconv",       true),
    ("calendar",    false),
    // System / process
    ("posix",       false),
    ("pcntl",       false),
    ("sysvmsg",     false),
    ("sysvsem",     false),
    ("sysvshm",     false),
    // Caching / OPcode
    ("opcache",     true),
    ("apcu",        false),
    // LDAP / Auth
    ("ldap",        false),
    // Mail
    ("imap",        false),
    // Dev / debug
    ("xdebug",      false),
    // Special (requires manual install)
    ("ioncube_loader", false),
];

// ── Serializable types ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub name:         String,
    pub enabled:      bool,
    pub description:  String,
    pub dev_only:     bool,
    pub category:     String,
    pub special_note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionToggle {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct OpcacheStats {
    pub enabled:         bool,
    pub hit_rate:        f64,
    pub memory_used_mb:  f64,
    pub memory_free_mb:  f64,
    pub cached_scripts:  i64,
    pub jit_enabled:     bool,
    pub jit_buffer_size: i64,
}

#[derive(Debug, Deserialize)]
pub struct JitToggle {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct PhpVersionRequest {
    pub php_version: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsReq {
    pub settings: std::collections::HashMap<String, String>,
}

// Site info returned by the checked-fetch helper
struct SiteInfo {
    unix_user:   String,
    php_version: String,
    domain:      String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/php/extensions/{site_id}",            get(list_extensions))
        .route("/api/v1/php/extensions/{site_id}/batch",      put(batch_toggle_extensions))
        .route("/api/v1/php/extensions/{site_id}/{ext_name}", put(toggle_extension))
        .route("/api/v1/php/opcache/{site_id}",               get(get_opcache_stats))
        .route("/api/v1/php/opcache/{site_id}/flush",         post(flush_opcache))
        .route("/api/v1/php/opcache/{site_id}/jit",           put(toggle_jit))
        .route("/api/v1/php/settings/{site_id}",              get(get_settings).put(update_settings))
        .route("/api/v1/php/info/{site_id}",                  get(php_info))
        .route("/api/v1/php/version/{site_id}",               put(change_php_version))
}

// ── Access helpers ────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn fetch_site_info(
    state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool,
) -> ApiResult<SiteInfo> {
    let site = sqlx::query!(
        "SELECT unix_user, php_version, domain FROM sites
         WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;
    Ok(SiteInfo {
        unix_user:   site.unix_user,
        php_version: site.php_version,
        domain:      site.domain,
    })
}

// ── Extension handlers ────────────────────────────────────────────────────────

async fn list_extensions(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ExtensionInfo>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let info = fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let enabled_set: std::collections::HashSet<String> = sqlx::query_scalar!(
        "SELECT extension_name FROM site_php_extensions WHERE site_id = $1 AND enabled = true",
        site_id
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();

    let loaded = read_fpm_extensions(&info.unix_user, &info.php_version);

    let extensions: Vec<ExtensionInfo> = AVAILABLE_EXTENSIONS.iter().map(|(name, default_on)| {
        let in_db  = enabled_set.contains(*name);
        let in_fpm = loaded.contains(&name.to_string());
        let enabled = if enabled_set.is_empty() { *default_on } else { in_db } || in_fpm;
        ExtensionInfo {
            name:         name.to_string(),
            enabled,
            description:  extension_description(name),
            dev_only:     *name == "xdebug",
            category:     extension_category(name),
            special_note: extension_special_note(name),
        }
    }).collect();

    Ok(Json(extensions))
}

async fn toggle_extension(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((site_id, ext_name)): Path<(Uuid, String)>,
    Json(req): Json<ExtensionToggle>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let ext_name = ext_name.to_lowercase();
    let valid_exts: Vec<&str> = AVAILABLE_EXTENSIONS.iter().map(|(n, _)| *n).collect();
    if !valid_exts.contains(&ext_name.as_str()) {
        return Err(ApiError::Validation(format!("Unknown extension: {}", ext_name)));
    }

    if ext_name == "xdebug" && req.enabled && !state.config.is_pro() {
        return Err(ApiError::Validation("xdebug can only be enabled in Pro/dev environments".into()));
    }

    sqlx::query!(
        "INSERT INTO site_php_extensions (site_id, extension_name, enabled)
         VALUES ($1, $2, $3)
         ON CONFLICT (site_id, extension_name) DO UPDATE SET enabled = $3",
        site_id, ext_name, req.enabled
    )
    .execute(&state.db)
    .await?;

    queue_job(&state, serde_json::json!({
        "type": "reload_php_pool",
        "site_id": site_id,
        "action": "toggle_extension",
        "extension": ext_name,
        "enabled": req.enabled,
    })).await;

    Ok(StatusCode::OK)
}

async fn batch_toggle_extensions(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let extensions = req.get("extensions")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let valid_exts: Vec<&str> = AVAILABLE_EXTENSIONS.iter().map(|(n, _)| *n).collect();

    for (ext_name, enabled_val) in &extensions {
        let ext = ext_name.to_lowercase();
        if !valid_exts.contains(&ext.as_str()) { continue; }
        let enabled = enabled_val.as_bool().unwrap_or(false);
        sqlx::query!(
            "INSERT INTO site_php_extensions (site_id, extension_name, enabled)
             VALUES ($1, $2, $3)
             ON CONFLICT (site_id, extension_name) DO UPDATE SET enabled = $3",
            site_id, ext, enabled
        )
        .execute(&state.db)
        .await?;
    }

    queue_job(&state, serde_json::json!({
        "type": "reload_php_pool",
        "site_id": site_id,
        "action": "batch_toggle_extensions",
    })).await;

    Ok(StatusCode::OK)
}

// ── OPcache handlers ──────────────────────────────────────────────────────────

async fn get_opcache_stats(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<OpcacheStats>> {
    let (user_id, is_admin) = caller(&claims)?;
    let info = fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let stats = read_real_opcache_stats(&info.unix_user, &info.domain).await;
    Ok(Json(stats))
}

async fn flush_opcache(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    queue_job(&state, serde_json::json!({
        "type": "flush_opcache",
        "site_id": site_id,
    })).await;

    Ok(StatusCode::ACCEPTED)
}

async fn toggle_jit(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<JitToggle>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    sqlx::query!(
        "INSERT INTO site_php_settings (site_id, setting_key, setting_value)
         VALUES ($1, 'opcache.jit', $2)
         ON CONFLICT (site_id, setting_key) DO UPDATE SET setting_value = $2",
        site_id,
        if req.enabled { "tracing" } else { "off" }
    )
    .execute(&state.db)
    .await?;

    queue_job(&state, serde_json::json!({
        "type": "reload_php_pool",
        "site_id": site_id,
        "action": "toggle_jit",
        "enabled": req.enabled,
    })).await;

    Ok(StatusCode::OK)
}

// ── Settings handlers ─────────────────────────────────────────────────────────

async fn get_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<std::collections::HashMap<String, String>>> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    // Start with defaults
    let mut settings: std::collections::HashMap<String, String> = PHP_SETTINGS_DEFAULTS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Override with DB values for this site
    let rows = sqlx::query!(
        "SELECT setting_key, setting_value FROM site_php_settings WHERE site_id = $1",
        site_id
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for row in rows {
        if settings.contains_key(&row.setting_key) {
            settings.insert(row.setting_key, row.setting_value);
        }
    }

    Ok(Json(settings))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<UpdateSettingsReq>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let allowed_keys: std::collections::HashSet<&str> =
        PHP_SETTINGS_DEFAULTS.iter().map(|(k, _)| *k).collect();

    for (key, value) in &req.settings {
        if !allowed_keys.contains(key.as_str()) {
            return Err(ApiError::Validation(format!("Setting '{}' is not allowed", key)));
        }
        // Generic length cap — no legitimate php.ini value exceeds 128 chars
        if value.len() > 128 {
            return Err(ApiError::Validation(format!("Value for '{}' exceeds 128 chars", key)));
        }
        // Basic value sanitisation — no shell metacharacters
        if value.contains(|c: char| matches!(c, ';' | '\n' | '\r' | '\0' | '"' | '\'' | '$' | '`' | '\\')) {
            return Err(ApiError::Validation(format!("Invalid value for '{}'", key)));
        }
        // Per-setting validation
        validate_setting_value(key, value)?;
        sqlx::query!(
            "INSERT INTO site_php_settings (site_id, setting_key, setting_value)
             VALUES ($1, $2, $3)
             ON CONFLICT (site_id, setting_key) DO UPDATE SET setting_value = $3",
            site_id, key, value
        )
        .execute(&state.db)
        .await?;
    }

    queue_job(&state, serde_json::json!({
        "type": "reload_php_pool",
        "site_id": site_id,
        "action": "update_settings",
    })).await;

    Ok(StatusCode::OK)
}

// ── phpinfo() ─────────────────────────────────────────────────────────────────

async fn php_info(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let info = fetch_site_info(&state, site_id, user_id, is_admin).await?;

    let php_bin = format!("/usr/bin/php{}", info.php_version);

    let output = tokio::process::Command::new("sudo")
        .args(["-u", &info.unix_user, &php_bin, "-r", "phpinfo();"])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("phpinfo exec failed: {}", e)))?;

    let raw_html = String::from_utf8_lossy(&output.stdout).to_string();
    let sanitized = sanitize_phpinfo(&raw_html);

    Ok(Json(serde_json::json!({ "html": sanitized })))
}

fn sanitize_phpinfo(raw: &str) -> String {
    let re_server = regex::Regex::new(r"(?s)<tr[^>]*>.*?_SERVER.*?</tr>")
        .unwrap_or_else(|_| regex::Regex::new(".^").unwrap());
    let mut s = re_server.replace_all(raw, "").to_string();

    let re_path = regex::Regex::new(r#"/(home|var|etc|usr|srv|opt)/[^\s<&"']*"#)
        .unwrap_or_else(|_| regex::Regex::new(".^").unwrap());
    s = re_path.replace_all(&s, "[PATH REDACTED]").to_string();
    s
}

// ── PHP version change ────────────────────────────────────────────────────────

async fn change_php_version(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
    Json(req): Json<PhpVersionRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    if !regex::Regex::new(r"^[78]\.[0-9]+$").unwrap().is_match(&req.php_version) {
        return Err(ApiError::Validation(format!("Invalid PHP version: {}", req.php_version)));
    }

    let result = sqlx::query!(
        "UPDATE sites SET php_version = $1, updated_at = NOW()
         WHERE id = $2 AND deleted_at IS NULL AND ($3::boolean OR owner_id = $4)",
        req.php_version, site_id, is_admin, user_id
    )
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound { resource: "site" });
    }

    queue_job(&state, serde_json::json!({
        "type": "change_php_version",
        "site_id": site_id,
        "version": req.php_version,
    })).await;

    Ok(StatusCode::OK)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Per-setting allowlist validation. Returns Err if value is out of range or invalid format.
fn validate_setting_value(key: &str, value: &str) -> ApiResult<()> {
    // Byte-size settings: must match digits + optional K/M/G suffix, capped at 4G (4096 MB).
    let byte_keys = ["memory_limit", "upload_max_filesize", "post_max_size"];
    if byte_keys.contains(&key) {
        let mb = parse_byte_size_mb(value)
            .ok_or_else(|| ApiError::Validation(format!("'{}' must be like 128M or 2G", key)))?;
        if mb == 0 || mb > 4096 {
            return Err(ApiError::Validation(format!("'{}' must be between 1M and 4G", key)));
        }
        return Ok(());
    }

    // Integer-bounded settings
    let int_bounds: &[(&str, i64, i64)] = &[
        ("max_execution_time",         0,     3600),
        ("max_input_time",             -1,    3600),
        ("max_file_uploads",           0,     1000),
        ("max_input_vars",             1,     100000),
        ("session.gc_maxlifetime",     60,    86400 * 30),
        ("opcache.memory_consumption", 8,     2048),
        ("opcache.revalidate_freq",    0,     3600),
    ];
    if let Some(&(_, min, max)) = int_bounds.iter().find(|(k, _, _)| *k == key) {
        let n: i64 = value.parse()
            .map_err(|_| ApiError::Validation(format!("'{}' must be an integer", key)))?;
        if n < min || n > max {
            return Err(ApiError::Validation(format!("'{}' must be between {} and {}", key, min, max)));
        }
        return Ok(());
    }

    // Toggles: On/Off/1/0/true/false/yes/no
    let toggle_keys = ["display_errors", "log_errors", "session.cookie_httponly",
                       "session.use_strict_mode", "opcache.enable", "opcache.validate_timestamps"];
    if toggle_keys.contains(&key) {
        let v = value.to_lowercase();
        if !matches!(v.as_str(), "on" | "off" | "1" | "0" | "true" | "false" | "yes" | "no") {
            return Err(ApiError::Validation(format!("'{}' must be On/Off/1/0", key)));
        }
        return Ok(());
    }

    // Enum settings — strict allowlist
    match key {
        "error_reporting" => {
            if !matches!(value, "production" | "development" | "strict") {
                return Err(ApiError::Validation("error_reporting must be production, development, or strict".into()));
            }
        }
        "session.save_handler" => {
            if !matches!(value, "files" | "redis") {
                return Err(ApiError::Validation("session.save_handler must be files or redis".into()));
            }
        }
        "opcache.jit" => {
            if !matches!(value, "off" | "tracing" | "function" | "0" | "1235") {
                return Err(ApiError::Validation("opcache.jit must be off, tracing, or function".into()));
            }
        }
        "date.timezone" => {
            // Must look like a valid timezone identifier: optional region/city/subcity
            // letters, digits, slashes, underscores, hyphens only — max 64 chars.
            if value.len() > 64 || value.is_empty() {
                return Err(ApiError::Validation("date.timezone must be 1-64 chars".into()));
            }
            if !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+')) {
                return Err(ApiError::Validation("date.timezone has invalid characters".into()));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Parse "128M", "2G", "512K", or plain bytes into MB. Returns None if malformed.
fn parse_byte_size_mb(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let (num_part, mult): (&str, u64) = if let Some(stripped) = s.strip_suffix(|c: char| matches!(c, 'K'|'k')) {
        (stripped, 0)  // K → fractions of MB, treat as 0 → invalid (we want at least 1M)
    } else if let Some(stripped) = s.strip_suffix(|c: char| matches!(c, 'M'|'m')) {
        (stripped, 1)
    } else if let Some(stripped) = s.strip_suffix(|c: char| matches!(c, 'G'|'g')) {
        (stripped, 1024)
    } else {
        (s, 1)  // assume MB if no suffix
    };
    let n: u64 = num_part.trim().parse().ok()?;
    Some(n.checked_mul(mult)?)
}

async fn queue_job(state: &AppState, job: serde_json::Value) {
    use redis::AsyncCommands;
    let mut conn = state.valkey.clone();
    let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
}

/// Get real OPcache stats from the site's PHP-FPM pool by writing a temporary
/// PHP script to the site's webroot, requesting it via localhost, and deleting it.
async fn read_real_opcache_stats(unix_user: &str, domain: &str) -> OpcacheStats {
    let zero = OpcacheStats {
        enabled: false, hit_rate: 0.0, memory_used_mb: 0.0,
        memory_free_mb: 0.0, cached_scripts: 0, jit_enabled: false, jit_buffer_size: 0,
    };

    // Try the pre-written stat file first (populated by orbit-agent if present)
    let stat_file = format!("/run/orbit/opcache/{}.json", unix_user);
    if let Ok(content) = std::fs::read_to_string(&stat_file) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            return parse_opcache_json(&v);
        }
    }

    // Temp PHP file → curl via FPM → real stats
    let webroot = format!("/home/{}/public_html", unix_user);
    if !std::path::Path::new(&webroot).is_dir() {
        return zero;
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".oc{:08x}.php", ts);
    let tmp_path  = format!("{}/{}", webroot, tmp_name);

    let script = concat!(
        "<?php\n",
        "$s=@opcache_get_status(false);\n",
        "if($s&&is_array($s)){\n",
        "  echo json_encode([\n",
        "    'enabled'=>true,\n",
        "    'hit_rate'=>round((float)($s['opcache_statistics']['opcache_hit_rate']??0),1),\n",
        "    'memory_used_mb'=>round((int)($s['memory_usage']['used_memory']??0)/1048576,1),\n",
        "    'memory_free_mb'=>round((int)($s['memory_usage']['free_memory']??0)/1048576,1),\n",
        "    'cached_scripts'=>(int)($s['opcache_statistics']['num_cached_scripts']??0),\n",
        "    'jit_enabled'=>(bool)($s['jit']['enabled']??false),\n",
        "    'jit_buffer_size'=>(int)($s['jit']['buffer_size']??0),\n",
        "  ]);\n",
        "}else{\n",
        "  echo json_encode(['enabled'=>false,'hit_rate'=>0.0,'memory_used_mb'=>0.0,'memory_free_mb'=>0.0,'cached_scripts'=>0,'jit_enabled'=>false,'jit_buffer_size'=>0]);\n",
        "}\n"
    );

    if std::fs::write(&tmp_path, script).is_err() {
        return zero;
    }

    let url = format!("http://127.0.0.1/{}", tmp_name);
    let curl = tokio::process::Command::new("curl")
        .args(["-s", "--max-time", "5", "--connect-timeout", "3",
               "-H", &format!("Host: {}", domain),
               &url])
        .output()
        .await;

    let _ = std::fs::remove_file(&tmp_path);

    if let Ok(out) = curl {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            return parse_opcache_json(&v);
        }
    }

    zero
}

fn parse_opcache_json(v: &serde_json::Value) -> OpcacheStats {
    OpcacheStats {
        enabled:         v["enabled"].as_bool().unwrap_or(false),
        hit_rate:        v["hit_rate"].as_f64().unwrap_or(0.0),
        memory_used_mb:  v["memory_used_mb"].as_f64().unwrap_or(0.0),
        memory_free_mb:  v["memory_free_mb"].as_f64().unwrap_or(0.0),
        cached_scripts:  v["cached_scripts"].as_i64().unwrap_or(0),
        jit_enabled:     v["jit_enabled"].as_bool().unwrap_or(false),
        jit_buffer_size: v["jit_buffer_size"].as_i64().unwrap_or(0),
    }
}

fn read_fpm_extensions(unix_user: &str, php_version: &str) -> std::collections::HashSet<String> {
    let pool_conf = format!("/etc/php/{}/fpm/pool.d/{}.conf", php_version, unix_user);
    let content = std::fs::read_to_string(&pool_conf).unwrap_or_default();
    let re = regex::Regex::new(r"php_admin_value\[extension\]\s*=\s*(.+)").unwrap();
    let mut exts = std::collections::HashSet::new();
    for cap in re.captures_iter(&content) {
        for ext in cap[1].split(',') {
            exts.insert(ext.trim().trim_end_matches(".so").to_string());
        }
    }
    exts
}

fn extension_description(name: &str) -> String {
    match name {
        "json"           => "JSON encoding and decoding",
        "mbstring"       => "Multibyte string functions (UTF-8)",
        "xml"            => "XML parsing and generation",
        "dom"            => "DOM document manipulation",
        "simplexml"      => "Simple XML element access",
        "tokenizer"      => "PHP source tokenizer",
        "ctype"          => "Character type checking functions",
        "fileinfo"       => "File MIME type detection",
        "pdo_mysql"      => "PDO MySQL/MariaDB database driver",
        "pdo_pgsql"      => "PDO PostgreSQL database driver",
        "pdo_sqlite"     => "PDO SQLite database driver",
        "mysqli"         => "MySQLi database driver",
        "redis"          => "Redis/Valkey client for PHP",
        "mongodb"        => "MongoDB database driver",
        "memcached"      => "Memcached distributed caching client",
        "curl"           => "HTTP client library for web requests",
        "soap"           => "SOAP web service client/server",
        "sockets"        => "Low-level socket communication",
        "openssl"        => "OpenSSL encryption and TLS support",
        "gd"             => "Image processing (GIF, JPEG, PNG)",
        "imagick"        => "ImageMagick image manipulation",
        "exif"           => "Read EXIF metadata from images",
        "bcmath"         => "Arbitrary precision mathematics",
        "gmp"            => "GNU Multiple Precision arithmetic",
        "sodium"         => "Libsodium modern cryptography",
        "hash"           => "Message digest and HMAC functions",
        "zip"            => "ZIP archive creation and extraction",
        "zlib"           => "Gzip compression and decompression",
        "bz2"            => "Bzip2 compression support",
        "intl"           => "Internationalization and locale support",
        "iconv"          => "Character set conversion",
        "calendar"       => "Calendar conversion functions",
        "posix"          => "POSIX process and user functions",
        "pcntl"          => "Process control (fork, signals)",
        "sysvmsg"        => "System V message queues",
        "sysvsem"        => "System V semaphores",
        "sysvshm"        => "System V shared memory",
        "opcache"        => "Opcode caching for PHP performance",
        "apcu"           => "APCu in-memory key-value cache",
        "ldap"           => "LDAP directory service client",
        "imap"           => "IMAP/POP3/NNTP mail access",
        "xdebug"         => "Debugging and profiling (dev only)",
        "ioncube_loader" => "IonCube Loader for encoded PHP files",
        _                => "",
    }.into()
}

fn extension_category(name: &str) -> String {
    match name {
        "json"|"mbstring"|"xml"|"dom"|"simplexml"|"tokenizer"|"ctype"|"fileinfo" => "core",
        "pdo_mysql"|"pdo_pgsql"|"pdo_sqlite"|"mysqli"|"redis"|"mongodb"|"memcached" => "database",
        "curl"|"soap"|"sockets"|"openssl" => "networking",
        "gd"|"imagick"|"exif" => "image",
        "bcmath"|"gmp"|"sodium"|"hash" => "math",
        "zip"|"zlib"|"bz2" => "compression",
        "intl"|"iconv"|"calendar" => "i18n",
        "posix"|"pcntl"|"sysvmsg"|"sysvsem"|"sysvshm" => "system",
        "opcache"|"apcu" => "caching",
        "ldap" => "auth",
        "imap" => "mail",
        "xdebug" => "debug",
        "ioncube_loader" => "special",
        _ => "other",
    }.into()
}

fn extension_special_note(name: &str) -> Option<String> {
    match name {
        "ioncube_loader" => Some(
            "Requires manual IonCube Loader installation. Download from ioncube.com.".into()
        ),
        _ => None,
    }
}

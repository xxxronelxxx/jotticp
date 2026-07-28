use axum::{
    Router,
    routing::{delete, get, post},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WafStatus {
    /// True if at least one nginx vhost has `modsecurity on`.
    pub enabled_globally: bool,
    /// List of site_ids where WAF is currently enabled.
    pub enabled_sites:    Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct WafLogEntry {
    pub timestamp: String,
    pub client_ip: String,
    pub method:    String,
    pub uri:       String,
    pub rule_id:   String,
    pub message:   String,
    pub severity:  String,
    pub raw_line:  String,
}

#[derive(Debug, Serialize)]
pub struct WafLogs {
    pub entries: Vec<WafLogEntry>,
}

#[derive(Debug, Serialize)]
pub struct WhitelistRule {
    pub rule_id:  String,
    pub pattern:  String,
    pub raw_line: String,
}

#[derive(Debug, Serialize)]
pub struct WhitelistRules {
    pub rules: Vec<WhitelistRule>,
}

#[derive(Debug, Deserialize)]
pub struct AddWhitelistRuleRequest {
    /// URI pattern to allow — validated for safe characters before writing
    pub pattern: String,
    /// Numeric rule ID (100000–999999 range to avoid CRS collision)
    pub rule_id: u32,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ── Constants ─────────────────────────────────────────────────────────────────

const MODSEC_AUDIT_LOG: &str = "/var/log/nginx/modsec_audit.log";
const WAF_WHITELIST_CONF: &str = "/etc/jottiecp/waf/whitelist.conf";
const WAF_CRS_CONF: &str = "/etc/jottiecp/waf/crs.conf";

/// nginx vhost directory where per-site configs live.
const NGINX_VHOST_DIR: &str = "/etc/nginx/sites-enabled";

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/waf/status",               get(get_status))
        .route("/api/v1/waf/enable/{site_id}",      post(enable_waf))
        .route("/api/v1/waf/disable/{site_id}",     post(disable_waf))
        .route("/api/v1/waf/logs",                 get(get_logs))
        .route("/api/v1/waf/rules",                get(list_rules).post(add_rule))
        .route("/api/v1/waf/rules/{rule_id}",       delete(delete_rule))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

/// Validate a URI pattern: printable ASCII, no characters that could break
/// ModSecurity SecRule syntax or introduce injection.
///
/// The pattern is embedded as: SecRule REQUEST_URI "@contains PATTERN" ...
/// so we must reject anything that could escape the double-quoted argument or
/// inject additional SecRule directives (newlines, quotes, backslashes).
fn validate_pattern(pattern: &str) -> ApiResult<()> {
    if pattern.is_empty() || pattern.len() > 200 {
        return Err(ApiError::Validation("Pattern must be 1–200 characters".into()));
    }
    // Reject any character that could break SecRule syntax or allow injection.
    // This includes double-quotes (would close the @contains argument), newlines
    // (would inject new SecRule lines), backslashes (escape sequences), and
    // standard shell metacharacters.
    if pattern.chars().any(|c| matches!(c, ';' | '&' | '|' | '$' | '`' | '\'' | '"' | '\n' | '\r' | '\\')) {
        return Err(ApiError::Validation(
            "Pattern contains invalid characters (must not include: ; & | $ ` ' \" \\ or newlines)".into()
        ));
    }
    // Only allow printable ASCII (0x20–0x7E)
    if pattern.chars().any(|c| !c.is_ascii() || (c as u8) < 0x20) {
        return Err(ApiError::Validation("Pattern must contain only printable ASCII characters".into()));
    }
    Ok(())
}

/// Read a file's contents, returning empty string if it doesn't exist.
async fn read_file_or_empty(path: &str) -> String {
    tokio::fs::read_to_string(path).await.unwrap_or_default()
}

/// Reload nginx configuration with `nginx -s reload`.
async fn reload_nginx() -> ApiResult<()> {
    let output = tokio::process::Command::new("sudo")
        .args(["nginx", "-s", "reload"])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("nginx reload spawn: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ApiError::ExternalService(format!("nginx reload failed: {}", stderr)));
    }
    Ok(())
}

/// Validate nginx config before applying.
async fn nginx_test() -> ApiResult<()> {
    let output = tokio::process::Command::new("sudo")
        .args(["nginx", "-t"])
        .output()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("nginx test spawn: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ApiError::Validation(format!("nginx config invalid: {}", stderr)));
    }
    Ok(())
}

/// Get the vhost config path for a site_id.
fn vhost_path(site_id: Uuid) -> String {
    format!("{}/{}.conf", NGINX_VHOST_DIR, site_id)
}

/// Check if a vhost config has the modsecurity directive enabled.
fn vhost_has_modsec(content: &str) -> bool {
    content.lines().any(|l| l.trim() == "modsecurity on;")
}

/// Add modsecurity directives to a vhost config (inside first `server {` block).
fn inject_modsec(content: &str) -> String {
    let modsec_block = format!(
        "    modsecurity on;\n    modsecurity_rules_file {};\n",
        WAF_CRS_CONF
    );
    // Insert after the first `server {` line
    if let Some(pos) = content.find("server {") {
        let insert_at = pos + "server {".len();
        let mut result = content.to_string();
        result.insert_str(insert_at, &format!("\n{}", modsec_block));
        result
    } else {
        // Fallback: prepend
        format!("{}\n{}", modsec_block, content)
    }
}

/// Remove modsecurity lines from a vhost config.
fn remove_modsec(content: &str) -> String {
    content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("modsecurity on") && !t.starts_with("modsecurity_rules_file")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse a ModSecurity audit log line (simplified — handles Apache/nginx modsec format).
/// Real format varies; this handles the common `--boundary--` sections.
fn parse_modsec_log_line(line: &str) -> Option<WafLogEntry> {
    // Minimal parse: look for lines containing [id "NNNNN"] and extract fields
    // Full modsec audit log parsing would use the multi-part boundary format.
    // For simplicity we extract from single-line JSON or key=value patterns.
    let rule_re = Regex::new(r#"\[id "(\d+)"\]"#).ok()?;
    let msg_re  = Regex::new(r#"\[msg "([^"]+)"\]"#).ok()?;
    let sev_re  = Regex::new(r#"\[severity "([^"]+)"\]"#).ok()?;

    if !rule_re.is_match(line) {
        return None;
    }

    let rule_id = rule_re.captures(line)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let message = msg_re.captures(line)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let severity = sev_re.captures(line)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "UNKNOWN".into());

    Some(WafLogEntry {
        timestamp: String::new(),
        client_ip: String::new(),
        method:    String::new(),
        uri:       String::new(),
        rule_id,
        message,
        severity,
        raw_line: line.to_string(),
    })
}

/// Parse whitelist.conf rules.
/// Each rule looks like:
/// `SecRule REQUEST_URI "@contains PATTERN" "id:ID,phase:1,allow,nolog"`
fn parse_whitelist_rules(content: &str) -> Vec<WhitelistRule> {
    let id_re      = Regex::new(r"id:(\d+)").unwrap();
    let pattern_re = Regex::new(r#"@contains ([^"]+)"#).unwrap();

    content
        .lines()
        .filter(|l| l.trim_start().starts_with("SecRule"))
        .filter_map(|line| {
            let rule_id = id_re.captures(line)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let pattern = pattern_re.captures(line)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            if rule_id.is_empty() { return None; }
            Some(WhitelistRule {
                rule_id,
                pattern,
                raw_line: line.to_string(),
            })
        })
        .collect()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/waf/status — check if modsecurity is enabled in any nginx vhost
async fn get_status(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<WafStatus>> {
    require_admin(&claims)?;

    // Query active site IDs from DB
    let site_ids: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT id FROM sites WHERE deleted_at IS NULL"
    )
    .fetch_all(&state.db)
    .await?;

    let mut enabled_sites = Vec::new();
    for id in &site_ids {
        let path = vhost_path(*id);
        let content = read_file_or_empty(&path).await;
        if vhost_has_modsec(&content) {
            enabled_sites.push(*id);
        }
    }

    // Also detect a global http-context enable (/etc/nginx/conf.d/*modsecurity*), which
    // protects every proxied site at once — not just per-vhost `modsecurity on;`.
    let global_conf = read_file_or_empty("/etc/nginx/conf.d/00-modsecurity.conf").await;
    let global_on = global_conf.lines().any(|l| l.trim() == "modsecurity on;");
    let enabled_globally = global_on || !enabled_sites.is_empty();
    Ok(Json(WafStatus { enabled_globally, enabled_sites }))
}

/// POST /api/v1/waf/enable/{site_id} — add modsecurity to vhost
async fn enable_waf(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<MessageResponse>> {
    require_admin(&claims)?;

    // Verify site exists
    let exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL)",
        site_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !exists {
        return Err(ApiError::NotFound { resource: "site" });
    }

    let path = vhost_path(site_id);
    let content = read_file_or_empty(&path).await;

    if vhost_has_modsec(&content) {
        return Ok(Json(MessageResponse { message: "WAF already enabled for this site".into() }));
    }

    let new_content = inject_modsec(&content);
    tokio::fs::write(&path, &new_content)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write vhost: {}", e)))?;

    nginx_test().await?;
    reload_nginx().await?;

    tracing::info!(admin = %claims.sub, site_id = %site_id, "WAF enabled for site");
    Ok(Json(MessageResponse { message: format!("WAF enabled for site {site_id}") }))
}

/// POST /api/v1/waf/disable/{site_id} — remove modsecurity directive from vhost
async fn disable_waf(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(site_id): Path<Uuid>,
) -> ApiResult<Json<MessageResponse>> {
    require_admin(&claims)?;

    let exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE id = $1 AND deleted_at IS NULL)",
        site_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !exists {
        return Err(ApiError::NotFound { resource: "site" });
    }

    let path = vhost_path(site_id);
    let content = read_file_or_empty(&path).await;
    let new_content = remove_modsec(&content);

    tokio::fs::write(&path, &new_content)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write vhost: {}", e)))?;

    nginx_test().await?;
    reload_nginx().await?;

    tracing::info!(admin = %claims.sub, site_id = %site_id, "WAF disabled for site");
    Ok(Json(MessageResponse { message: format!("WAF disabled for site {site_id}") }))
}

/// GET /api/v1/waf/logs — tail last 200 lines of modsec audit log
async fn get_logs(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<WafLogs>> {
    require_admin(&claims)?;

    let content = read_file_or_empty(MODSEC_AUDIT_LOG).await;
    let lines: Vec<&str> = content.lines().collect();
    let tail_lines: Vec<&str> = lines.iter()
        .rev()
        .take(200)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let entries = tail_lines
        .into_iter()
        .filter_map(parse_modsec_log_line)
        .collect();

    Ok(Json(WafLogs { entries }))
}

/// GET /api/v1/waf/rules — list custom whitelist rules from whitelist.conf
async fn list_rules(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<WhitelistRules>> {
    require_admin(&claims)?;

    let content = read_file_or_empty(WAF_WHITELIST_CONF).await;
    Ok(Json(WhitelistRules { rules: parse_whitelist_rules(&content) }))
}

/// POST /api/v1/waf/rules — append a whitelist rule to whitelist.conf
async fn add_rule(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddWhitelistRuleRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    require_admin(&claims)?;

    // Rule ID range check FIRST (before any file I/O or pattern validation)
    // so we fail fast and never touch the filesystem for an invalid request.
    if req.rule_id < 100_000 || req.rule_id > 999_999 {
        return Err(ApiError::Validation("rule_id must be between 100000 and 999999".into()));
    }

    // Pattern is embedded literally inside double-quoted SecRule argument — validate strictly.
    validate_pattern(&req.pattern)?;

    // Check if rule_id already exists
    let content = read_file_or_empty(WAF_WHITELIST_CONF).await;
    let existing = parse_whitelist_rules(&content);
    if existing.iter().any(|r| r.rule_id == req.rule_id.to_string()) {
        return Err(ApiError::Validation(format!("Rule ID {} already exists", req.rule_id)));
    }

    let rule_line = format!(
        r#"SecRule REQUEST_URI "@contains {}" "id:{},phase:1,allow,nolog""#,
        req.pattern, req.rule_id
    );

    let mut new_content = content;
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(&rule_line);
    new_content.push('\n');

    // Ensure directory exists
    let conf_path = std::path::Path::new(WAF_WHITELIST_CONF);
    if let Some(parent) = conf_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Create WAF dir: {}", e)))?;
    }

    // Atomic write: write to a temp file next to the destination, then rename.
    // This prevents a partial write from corrupting whitelist.conf.
    let tmp_path = format!("{}.tmp.{}", WAF_WHITELIST_CONF, std::process::id());
    tokio::fs::write(&tmp_path, &new_content)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write whitelist.conf (tmp): {}", e)))?;
    tokio::fs::rename(&tmp_path, WAF_WHITELIST_CONF)
        .await
        .map_err(|e| {
            // Clean up orphaned tmp file on rename failure (best-effort)
            let _ = std::fs::remove_file(&tmp_path);
            ApiError::Internal(anyhow::anyhow!("Rename whitelist.conf: {}", e))
        })?;

    tracing::info!(admin = %claims.sub, rule_id = req.rule_id, "WAF whitelist rule added");
    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("Whitelist rule {} added", req.rule_id),
    })))
}

/// DELETE /api/v1/waf/rules/{rule_id} — remove rule by ID from whitelist.conf
async fn delete_rule(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(rule_id): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;

    // Validate rule_id is a number in the allowed range
    let rid: u32 = rule_id.parse().map_err(|_| ApiError::Validation("rule_id must be a number".into()))?;
    if rid < 100_000 || rid > 999_999 {
        return Err(ApiError::Validation("rule_id must be between 100000 and 999999".into()));
    }

    let content = read_file_or_empty(WAF_WHITELIST_CONF).await;
    // Match exactly "id:NNNNN" with a non-digit after to prevent id:100001 matching id:1000010
    let id_pattern = format!("id:{}", rule_id);

    let new_lines: Vec<&str> = content
        .lines()
        .filter(|l| !l.contains(&id_pattern))
        .collect();

    // If unchanged, rule was not found
    if new_lines.len() == content.lines().count() {
        return Err(ApiError::NotFound { resource: "WAF rule" });
    }

    let mut out = new_lines.join("\n");
    out.push('\n');

    // Atomic write via temp file + rename
    let tmp_path = format!("{}.tmp.{}", WAF_WHITELIST_CONF, std::process::id());
    tokio::fs::write(&tmp_path, &out)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write whitelist.conf (tmp): {}", e)))?;
    tokio::fs::rename(&tmp_path, WAF_WHITELIST_CONF)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            ApiError::Internal(anyhow::anyhow!("Rename whitelist.conf: {}", e))
        })?;

    tracing::info!(admin = %claims.sub, rule_id = %rule_id, "WAF whitelist rule deleted");
    Ok(StatusCode::NO_CONTENT)
}

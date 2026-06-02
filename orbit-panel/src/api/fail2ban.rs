use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Fail2banStatus {
    pub running:     bool,
    pub active_unit: String,
}

#[derive(Debug, Serialize)]
pub struct JailList {
    pub jails: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JailDetail {
    pub name:           String,
    pub currently_failed: i64,
    pub total_failed:   i64,
    pub currently_banned: i64,
    pub total_banned:   i64,
    pub banned_ips:     Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BannedIpEntry {
    pub jail: String,
    pub ip:   String,
}

#[derive(Debug, Serialize)]
pub struct BannedList {
    pub entries: Vec<BannedIpEntry>,
}

#[derive(Debug, Deserialize)]
pub struct UnbanRequest {
    pub ip: String,
}

#[derive(Debug, Deserialize)]
pub struct MaxRetryRequest {
    pub maxretry: u32,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/fail2ban/status",                    get(get_status))
        .route("/api/v1/fail2ban/jails",                     get(list_jails))
        .route("/api/v1/fail2ban/jails/{jail}",               get(get_jail))
        .route("/api/v1/fail2ban/jails/{jail}/unban",         post(unban_ip))
        .route("/api/v1/fail2ban/jails/{jail}/maxretry",      put(set_maxretry))
        .route("/api/v1/fail2ban/banned",                    get(list_all_banned))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

fn validate_ip(ip: &str) -> ApiResult<()> {
    // Shell metacharacter check first
    if ip.chars().any(|c| matches!(c, ';' | '&' | '|' | '$' | '`' | '\\' | '\'' | '"' | '\n' | '\r' | ' ' | '\t' | '{' | '}')) {
        return Err(ApiError::Validation("IP address contains invalid characters".into()));
    }

    // IPv4 / IPv4 CIDR with octet and prefix range validation
    let ipv4_re = Regex::new(r"^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:/(\d{1,2}))?$").unwrap();
    if let Some(caps) = ipv4_re.captures(ip) {
        for i in 1..=4 {
            let octet: u32 = caps.get(i).unwrap().as_str().parse().unwrap_or(256);
            if octet > 255 {
                return Err(ApiError::Validation(format!("Invalid IPv4 octet in: {ip}")));
            }
        }
        if let Some(prefix) = caps.get(5) {
            let len: u32 = prefix.as_str().parse().unwrap_or(33);
            if len > 32 {
                return Err(ApiError::Validation(format!("Invalid IPv4 prefix length in: {ip}")));
            }
        }
        return Ok(());
    }

    // IPv6 / IPv6 CIDR
    let ipv6_re = Regex::new(r"^[0-9a-fA-F:]+(?:/(\d{1,3}))?$").unwrap();
    if let Some(caps) = ipv6_re.captures(ip) {
        if !ip.contains(':') {
            return Err(ApiError::Validation(format!("Invalid IP address: {ip}")));
        }
        if let Some(prefix) = caps.get(1) {
            let len: u32 = prefix.as_str().parse().unwrap_or(129);
            if len > 128 {
                return Err(ApiError::Validation(format!("Invalid IPv6 prefix length in: {ip}")));
            }
        }
        return Ok(());
    }

    Err(ApiError::Validation(format!("Invalid IP address: {ip}")))
}

/// Validate a jail name: alphanumeric, dash, underscore only; max 64 chars.
fn validate_jail_name(jail: &str) -> ApiResult<()> {
    let re = Regex::new(r"^[a-zA-Z0-9_-]{1,64}$").unwrap();
    if !re.is_match(jail) {
        return Err(ApiError::Validation(format!("Invalid jail name: {jail}")));
    }
    Ok(())
}

/// Run `fail2ban-client ...` and return stdout.
async fn f2b(args: &[&str]) -> anyhow::Result<String> {
    let mut cmd = tokio::process::Command::new("fail2ban-client");
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(anyhow::anyhow!("fail2ban-client error: {}", stderr))
    }
}

/// Run `systemctl is-active fail2ban`.
async fn systemctl_is_active(unit: &str) -> bool {
    tokio::process::Command::new("systemctl")
        .args(["is-active", unit])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse `fail2ban-client status` output to extract the jail list.
/// Output looks like:
/// ```
/// Status
/// |- Number of jail:	2
/// `- Jail list:	sshd, orbitcp-admin
/// ```
fn parse_jail_list(output: &str) -> Vec<String> {
    for line in output.lines() {
        if line.contains("Jail list:") {
            if let Some(list_part) = line.splitn(2, ':').nth(1) {
                return list_part
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    vec![]
}

/// Parse `fail2ban-client status JAIL` output.
/// Extracts currently failed, total failed, currently banned, total banned, banned IPs.
fn parse_jail_detail(name: &str, output: &str) -> JailDetail {
    let mut currently_failed = 0i64;
    let mut total_failed = 0i64;
    let mut currently_banned = 0i64;
    let mut total_banned = 0i64;
    let mut banned_ips = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Currently failed:") {
            currently_failed = line.split(':').nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        } else if line.starts_with("Total failed:") {
            total_failed = line.split(':').nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        } else if line.starts_with("Currently banned:") {
            currently_banned = line.split(':').nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        } else if line.starts_with("Total banned:") {
            total_banned = line.split(':').nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        } else if line.starts_with("Banned IP list:") {
            if let Some(ip_part) = line.splitn(2, ':').nth(1) {
                banned_ips = ip_part
                    .split_whitespace()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }

    JailDetail {
        name: name.to_string(),
        currently_failed,
        total_failed,
        currently_banned,
        total_banned,
        banned_ips,
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/fail2ban/status — is fail2ban running?
async fn get_status(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Fail2banStatus>> {
    require_admin(&claims)?;
    let running = systemctl_is_active("fail2ban").await;
    Ok(Json(Fail2banStatus {
        running,
        active_unit: "fail2ban".into(),
    }))
}

/// GET /api/v1/fail2ban/jails — list all jails
async fn list_jails(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<JailList>> {
    require_admin(&claims)?;

    let out = f2b(&["status"])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    Ok(Json(JailList { jails: parse_jail_list(&out) }))
}

/// GET /api/v1/fail2ban/jails/{jail} — stats + banned IPs for one jail
async fn get_jail(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(jail): Path<String>,
) -> ApiResult<Json<JailDetail>> {
    require_admin(&claims)?;
    validate_jail_name(&jail)?;

    let out = f2b(&["status", &jail])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    Ok(Json(parse_jail_detail(&jail, &out)))
}

/// POST /api/v1/fail2ban/jails/{jail}/unban — unban an IP from a specific jail
async fn unban_ip(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(jail): Path<String>,
    Json(req): Json<UnbanRequest>,
) -> ApiResult<Json<MessageResponse>> {
    require_admin(&claims)?;
    validate_jail_name(&jail)?;
    validate_ip(&req.ip)?;

    f2b(&["set", &jail, "unbanip", &req.ip])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, jail = %jail, ip = %req.ip, "IP unbanned via fail2ban");
    Ok(Json(MessageResponse {
        message: format!("IP {} unbanned from jail {}", req.ip, jail),
    }))
}

/// GET /api/v1/fail2ban/banned — aggregated list of all currently banned IPs
async fn list_all_banned(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<BannedList>> {
    require_admin(&claims)?;

    let status_out = f2b(&["status"])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;
    let jails = parse_jail_list(&status_out);

    let mut entries = Vec::new();
    for jail in jails {
        if let Ok(out) = f2b(&["status", &jail]).await {
            let detail = parse_jail_detail(&jail, &out);
            for ip in detail.banned_ips {
                entries.push(BannedIpEntry { jail: jail.clone(), ip });
            }
        }
    }

    Ok(Json(BannedList { entries }))
}

/// PUT /api/v1/fail2ban/jails/{jail}/maxretry — update maxretry for a jail
///
/// Writes `/etc/fail2ban/jail.d/orbitcp-JAIL.conf` then reloads fail2ban.
async fn set_maxretry(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(jail): Path<String>,
    Json(req): Json<MaxRetryRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    require_admin(&claims)?;
    validate_jail_name(&jail)?;

    if req.maxretry == 0 || req.maxretry > 100 {
        return Err(ApiError::Validation("maxretry must be between 1 and 100".into()));
    }

    // Double-check: jail name must not contain path separators even after passing
    // validate_jail_name (belt-and-braces; the regex already forbids '/' but be explicit).
    if jail.contains('/') || jail.contains("..") {
        return Err(ApiError::Validation("Invalid jail name".into()));
    }

    let conf_path = format!("/etc/fail2ban/jail.d/orbitcp-{jail}.conf");
    let conf_content = format!(
        "[{jail}]\nenabled = true\nmaxretry = {}\n",
        req.maxretry
    );

    tokio::fs::write(&conf_path, conf_content)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Write jail config: {}", e)))?;

    // Reload fail2ban to apply the new config
    f2b(&["reload"])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(
        admin = %claims.sub, jail = %jail, maxretry = req.maxretry,
        "fail2ban maxretry updated"
    );
    Ok((StatusCode::OK, Json(MessageResponse {
        message: format!("maxretry for jail '{}' set to {}", jail, req.maxretry),
    })))
}

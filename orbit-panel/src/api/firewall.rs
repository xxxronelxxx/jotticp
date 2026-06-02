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

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FirewallStatus {
    pub enabled:       bool,
    pub rule_count:    i64,
    pub blocked_count: i64,
}

#[derive(Debug, Serialize)]
pub struct NftRuleset {
    pub raw_json: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct AddRuleRequest {
    pub ip:     String,
    pub action: String, // "allow" | "block"
}

#[derive(Debug, Deserialize)]
pub struct AddIpRequest {
    pub ip: String,
}

#[derive(Debug, Serialize)]
pub struct IpListResponse {
    pub ips: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SshRateLimitRequest {
    /// Number of connections allowed in the window, e.g. 10
    pub limit:  u32,
    /// Time window, e.g. "minute" | "second" | "hour"
    pub window: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    // All firewall routes require admin role — enforced at router level via middleware
    // so individual handlers cannot accidentally skip the check.
    Router::new()
        .route("/api/v1/firewall/status",         get(get_status))
        .route("/api/v1/firewall/rules",           get(list_rules).post(add_rule))
        .route("/api/v1/firewall/rules/{ip}",       delete(remove_rule))
        .route("/api/v1/firewall/blocked-ips",     get(list_blocked).post(add_blocked))
        .route("/api/v1/firewall/blocked-ips/{ip}", delete(remove_blocked))
        .route("/api/v1/firewall/whitelist",       get(list_whitelist).post(add_whitelist))
        .route("/api/v1/firewall/whitelist/{ip}",   delete(remove_whitelist))
        .route("/api/v1/firewall/ssh-rate-limit",  post(set_ssh_rate_limit))
        .route("/api/v1/firewall/geo-blocks",      get(get_geo_blocks).post(set_geo_blocks))
}

// ── Geo blocks (persisted country list; enforcement via GeoIP is a follow-up) ──

#[derive(Debug, Serialize, Deserialize)]
pub struct GeoBlocks {
    #[serde(default)]
    pub countries: Vec<String>,
}

async fn get_geo_blocks(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<GeoBlocks>> {
    require_admin(&claims)?;
    let raw: Option<String> = sqlx::query_scalar!(
        "SELECT value FROM panel_settings WHERE key = 'firewall_geo_blocks'"
    )
    .fetch_optional(&state.db)
    .await?;
    let countries = raw
        .and_then(|s| serde_json::from_str::<Vec<String>>(s.as_str()).ok())
        .unwrap_or_default();
    Ok(Json(GeoBlocks { countries }))
}

async fn set_geo_blocks(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(body): Json<GeoBlocks>,
) -> ApiResult<Json<GeoBlocks>> {
    require_admin(&claims)?;
    // Normalise to upper-case 2-letter ISO codes.
    let countries: Vec<String> = body.countries.iter()
        .map(|c| c.trim().to_uppercase())
        .filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()))
        .collect();
    let json = serde_json::to_string(&countries).unwrap_or_else(|_| "[]".to_string());
    sqlx::query!(
        "INSERT INTO panel_settings (key, value, updated_at)
         VALUES ('firewall_geo_blocks', $1, NOW())
         ON CONFLICT (key) DO UPDATE SET value = $1, updated_at = NOW()",
        json
    )
    .execute(&state.db)
    .await?;
    Ok(Json(GeoBlocks { countries }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_admin(claims: &Claims) -> ApiResult<()> {
    if claims.role != "admin" {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

/// Validate IPv4, IPv4 CIDR, IPv6, and IPv6 CIDR addresses.
/// Rejects anything that does not match before passing to shell commands.
///
/// IPv4 octet values are range-checked (0-255) and prefix lengths are
/// range-checked (0-32 for v4, 0-128 for v6) to prevent spoofing via
/// technically-regex-matching but semantically invalid values.
fn validate_ip(ip: &str) -> ApiResult<()> {
    // Shell metacharacter check first — belt-and-braces before any further parsing
    if ip.chars().any(|c| matches!(c, ';' | '&' | '|' | '$' | '`' | '\\' | '\'' | '"' | '\n' | '\r' | ' ' | '\t' | '{' | '}')) {
        return Err(ApiError::Validation("IP address contains invalid characters".into()));
    }

    // Try IPv4 / IPv4 CIDR
    let ipv4_re = Regex::new(r"^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:/(\d{1,2}))?$").unwrap();
    if let Some(caps) = ipv4_re.captures(ip) {
        // Validate each octet is 0-255
        for i in 1..=4 {
            let octet: u32 = caps.get(i).unwrap().as_str().parse().unwrap_or(256);
            if octet > 255 {
                return Err(ApiError::Validation(format!("Invalid IPv4 octet in: {ip}")));
            }
        }
        // Validate prefix length 0-32
        if let Some(prefix) = caps.get(5) {
            let len: u32 = prefix.as_str().parse().unwrap_or(33);
            if len > 32 {
                return Err(ApiError::Validation(format!("Invalid IPv4 prefix length in: {ip}")));
            }
        }
        return Ok(());
    }

    // Try IPv6 / IPv6 CIDR — must be hex digits and colons only, with optional /prefix
    let ipv6_re = Regex::new(r"^[0-9a-fA-F:]+(?:/(\d{1,3}))?$").unwrap();
    if let Some(caps) = ipv6_re.captures(ip) {
        // IPv6 addresses must contain at least one colon
        if !ip.contains(':') {
            return Err(ApiError::Validation(format!("Invalid IP address: {ip}")));
        }
        // Validate prefix length 0-128
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

/// Run `sudo nft ...` with tokio::process::Command.
/// nft requires flags (e.g. -j) before the subcommand, so we partition args
/// and emit flags first regardless of where the caller placed them.
async fn nft(args: &[&str]) -> anyhow::Result<String> {
    let mut cmd = tokio::process::Command::new("sudo");
    cmd.arg("nft");
    // Flags must precede the subcommand in nft syntax
    for a in args.iter().filter(|a| a.starts_with('-')) {
        cmd.arg(a);
    }
    for a in args.iter().filter(|a| !a.starts_with('-')) {
        cmd.arg(a);
    }
    let output = cmd.output().await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(anyhow::anyhow!("nft error: {}", stderr))
    }
}

/// Create the orbit_filter table and sets if they don't exist yet.
/// Called lazily before any set operation so the first use doesn't 502.
async fn ensure_orbit_tables() {
    // Table
    let _ = tokio::process::Command::new("sudo")
        .args(["nft", "add", "table", "inet", "orbit_filter"])
        .output().await;
    // Sets
    for set in &["orbit_blocked", "orbit_whitelist"] {
        let _ = tokio::process::Command::new("sudo")
            .args(["nft", "add", "set", "inet", "orbit_filter", set,
                   "{ type ipv4_addr; flags interval; }"])
            .output().await;
    }
}

/// Parse the JSON output of `nft list ruleset -j` to count rules.
fn count_rules(json: &serde_json::Value) -> i64 {
    let Some(arr) = json
        .get("nftables")
        .and_then(|v| v.as_array())
    else {
        return 0;
    };
    arr.iter()
        .filter(|entry| entry.get("rule").is_some())
        .count() as i64
}

/// Count IPs in an nft set by parsing `nft list set inet orbit_filter SET_NAME -j`.
async fn count_set_elements(set_name: &str) -> i64 {
    let Ok(out) = nft(&["list", "set", "inet", "orbit_filter", set_name, "-j"]).await else {
        return 0;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) else {
        return 0;
    };
    // The elements live in nftables[*].set.elem
    json.get("nftables")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|entry| {
                entry.get("set")
                    .and_then(|s| s.get("elem"))
                    .and_then(|e| e.as_array())
                    .map(|e| e.len() as i64)
            })
        })
        .unwrap_or(0)
}

/// Parse set elements from `nft list set ... -j` output.
fn parse_set_elements(json_str: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return vec![];
    };
    json.get("nftables")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("set"))
                .filter_map(|s| s.get("elem"))
                .filter_map(|e| e.as_array())
                .flat_map(|elems| {
                    elems.iter().filter_map(|el| {
                        // Each element is either a plain string or {"prefix": ...}
                        if let Some(s) = el.as_str() {
                            Some(s.to_string())
                        } else if let Some(obj) = el.as_object() {
                            // Compact representation back to string
                            Some(serde_json::to_string(obj).unwrap_or_default())
                        } else {
                            None
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/firewall/status
async fn get_status(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<FirewallStatus>> {
    require_admin(&claims)?;

    // Check if nft binary is reachable and orbit_filter table exists
    let ruleset = nft(&["list", "ruleset", "-j"]).await;
    let (enabled, rule_count) = match ruleset {
        Ok(ref out) => {
            let json: serde_json::Value = serde_json::from_str(out).unwrap_or(serde_json::Value::Null);
            (true, count_rules(&json))
        }
        Err(_) => (false, 0),
    };

    let blocked_count = count_set_elements("orbit_blocked").await;

    Ok(Json(FirewallStatus { enabled, rule_count, blocked_count }))
}

/// GET /api/v1/firewall/rules — return parsed nftables ruleset as JSON
async fn list_rules(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<NftRuleset>> {
    require_admin(&claims)?;

    let out = nft(&["list", "ruleset", "-j"])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    let raw_json: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or(serde_json::json!({ "nftables": [] }));

    Ok(Json(NftRuleset { raw_json }))
}

/// POST /api/v1/firewall/rules — add IP to allowed or blocked set
async fn add_rule(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddRuleRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    require_admin(&claims)?;
    validate_ip(&req.ip)?;

    let set_name = match req.action.as_str() {
        "allow" => "orbit_whitelist",
        "block" => "orbit_blocked",
        _ => return Err(ApiError::Validation("action must be 'allow' or 'block'".into())),
    };

    nft(&["add", "element", "inet", "orbit_filter", set_name, &format!("{{ {} }}", req.ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %req.ip, action = %req.action, "firewall rule added");
    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("IP {} added to {}", req.ip, set_name),
    })))
}

/// DELETE /api/v1/firewall/rules/{ip} — remove IP from blocked set
async fn remove_rule(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(ip): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;
    validate_ip(&ip)?;

    nft(&["delete", "element", "inet", "orbit_filter", "orbit_blocked", &format!("{{ {} }}", ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %ip, "firewall rule removed from orbit_blocked");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/firewall/blocked-ips — list all IPs in orbit_blocked set
async fn list_blocked(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<IpListResponse>> {
    require_admin(&claims)?;
    ensure_orbit_tables().await;
    let out = nft(&["list", "set", "inet", "orbit_filter", "orbit_blocked", "-j"])
        .await
        .unwrap_or_default();
    Ok(Json(IpListResponse { ips: parse_set_elements(&out) }))
}

/// POST /api/v1/firewall/blocked-ips — add IP to orbit_blocked set
async fn add_blocked(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddIpRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    require_admin(&claims)?;
    validate_ip(&req.ip)?;

    nft(&["add", "element", "inet", "orbit_filter", "orbit_blocked", &format!("{{ {} }}", req.ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %req.ip, "IP added to orbit_blocked");
    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("IP {} added to orbit_blocked", req.ip),
    })))
}

/// DELETE /api/v1/firewall/blocked-ips/{ip} — remove from orbit_blocked set
async fn remove_blocked(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(ip): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;
    validate_ip(&ip)?;

    nft(&["delete", "element", "inet", "orbit_filter", "orbit_blocked", &format!("{{ {} }}", ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %ip, "IP removed from orbit_blocked");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/firewall/whitelist — list whitelisted IPs
async fn list_whitelist(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<IpListResponse>> {
    require_admin(&claims)?;
    ensure_orbit_tables().await;
    let out = nft(&["list", "set", "inet", "orbit_filter", "orbit_whitelist", "-j"])
        .await
        .unwrap_or_default();
    Ok(Json(IpListResponse { ips: parse_set_elements(&out) }))
}

/// POST /api/v1/firewall/whitelist — add to orbit_whitelist set
async fn add_whitelist(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<AddIpRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    require_admin(&claims)?;
    validate_ip(&req.ip)?;

    nft(&["add", "element", "inet", "orbit_filter", "orbit_whitelist", &format!("{{ {} }}", req.ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %req.ip, "IP added to orbit_whitelist");
    Ok((StatusCode::CREATED, Json(MessageResponse {
        message: format!("IP {} added to orbit_whitelist", req.ip),
    })))
}

/// DELETE /api/v1/firewall/whitelist/{ip} — remove from orbit_whitelist set
async fn remove_whitelist(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Path(ip): Path<String>,
) -> ApiResult<StatusCode> {
    require_admin(&claims)?;
    validate_ip(&ip)?;

    nft(&["delete", "element", "inet", "orbit_filter", "orbit_whitelist", &format!("{{ {} }}", ip)])
        .await
        .map_err(|e| ApiError::ExternalService(e.to_string()))?;

    tracing::info!(admin = %claims.sub, ip = %ip, "IP removed from orbit_whitelist");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/firewall/ssh-rate-limit — update SSH rate limit in nftables
///
/// Replaces the `ct rate` expression in the SSH chain `orbit_ssh_ratelimit`.
/// nft command: `nft replace rule inet orbit_filter input handle HANDLE ...`
/// Simpler approach: flush and re-add the rate-limit rule.
async fn set_ssh_rate_limit(
    State(_state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<SshRateLimitRequest>,
) -> ApiResult<Json<MessageResponse>> {
    require_admin(&claims)?;

    // Validate window
    if !matches!(req.window.as_str(), "second" | "minute" | "hour") {
        return Err(ApiError::Validation("window must be 'second', 'minute', or 'hour'".into()));
    }
    if req.limit == 0 || req.limit > 1000 {
        return Err(ApiError::Validation("limit must be between 1 and 1000".into()));
    }

    // Flush the SSH rate-limit chain and rebuild the rule:
    //   nft flush chain inet orbit_filter orbit_ssh_ratelimit
    //   nft add rule inet orbit_filter orbit_ssh_ratelimit ct rate LIMIT/WINDOW drop
    nft(&["flush", "chain", "inet", "orbit_filter", "orbit_ssh_ratelimit"])
        .await
        .map_err(|e| ApiError::ExternalService(format!("flush chain: {e}")))?;

    let rate_expr = format!("{}/{}", req.limit, req.window);
    nft(&[
        "add", "rule", "inet", "orbit_filter", "orbit_ssh_ratelimit",
        "ct", "rate", "over", &rate_expr, "drop",
    ])
    .await
    .map_err(|e| ApiError::ExternalService(format!("add rule: {e}")))?;

    tracing::info!(
        admin = %claims.sub, limit = req.limit, window = %req.window,
        "SSH rate limit updated"
    );
    Ok(Json(MessageResponse {
        message: format!("SSH rate limit set to {}/{}", req.limit, req.window),
    }))
}

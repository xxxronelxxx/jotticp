use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
    http::StatusCode,
    Json,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Axum middleware: block admin-role requests from IPs not in the allowlist.
///
/// Loaded from `panel_settings` where key = 'admin_ip_allowlist'.
/// Value is comma-separated CIDR list (e.g. "203.0.113.0/24,10.0.0.1").
/// Empty string = no restriction (allow all).
///
/// The allowlist is evaluated only when the JWT role == "admin".
/// Non-admin requests always pass through so client/reseller portals work.
pub async fn admin_ip_check(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::AppState>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    // Only restrict routes that require admin JWT (check Authorization header)
    let is_admin_request = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|tok| crate::api::auth::decode_jwt(tok, &state.jwt_decoding_key).ok())
        .map(|c| c.role == "admin")
        .unwrap_or(false);

    if !is_admin_request {
        return next.run(req).await;
    }

    // Fetch allowlist from DB (cache invalidation: just read every request — it's one row)
    let allowlist: String = sqlx::query_scalar!(
        "SELECT value FROM panel_settings WHERE key = 'admin_ip_allowlist'"
    )
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None)
    .unwrap_or_default();

    if allowlist.trim().is_empty() {
        // No restriction configured — allow all admin access
        return next.run(req).await;
    }

    let client_ip = peer.ip();

    let allowed = allowlist
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .any(|cidr| ip_in_cidr(client_ip, cidr));

    if allowed {
        next.run(req).await
    } else {
        tracing::warn!(
            client_ip = %client_ip,
            "Admin access denied — IP not in allowlist"
        );
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "ip_not_allowed",
                "message": "Your IP address is not authorised to access the admin panel."
            })),
        )
            .into_response()
    }
}

/// Check if `ip` falls within a CIDR range string like "203.0.113.0/24" or "10.0.0.5".
fn ip_in_cidr(ip: IpAddr, cidr: &str) -> bool {
    if let Some((addr_str, prefix_str)) = cidr.split_once('/') {
        let prefix_len: u8 = match prefix_str.parse() {
            Ok(p) => p,
            Err(_) => return false,
        };
        match (ip, addr_str.parse::<IpAddr>().ok()) {
            (IpAddr::V4(client), Some(IpAddr::V4(net))) => {
                if prefix_len > 32 { return false; }
                let mask = if prefix_len == 0 { 0u32 } else { u32::MAX << (32 - prefix_len) };
                (u32::from(client) & mask) == (u32::from(net) & mask)
            }
            (IpAddr::V6(client), Some(IpAddr::V6(net))) => {
                if prefix_len > 128 { return false; }
                ipv6_in_range(client, net, prefix_len)
            }
            _ => false,
        }
    } else {
        // No prefix — exact match
        cidr.parse::<IpAddr>().map(|a| a == ip).unwrap_or(false)
    }
}

fn ipv6_in_range(client: Ipv6Addr, net: Ipv6Addr, prefix: u8) -> bool {
    let c = u128::from(client);
    let n = u128::from(net);
    let mask = if prefix == 0 { 0u128 } else { u128::MAX << (128 - prefix) };
    (c & mask) == (n & mask)
}

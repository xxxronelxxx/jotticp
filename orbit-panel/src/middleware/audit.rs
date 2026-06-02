use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, Method},
    middleware::Next,
    response::Response,
};
use std::{net::SocketAddr, sync::Arc, time::Instant};
use uuid::Uuid;

use crate::AppState;
use crate::api::auth::decode_jwt;

/// Audit middleware: records every state-changing API call to the audit_log table.
///
/// Only logs POST / PUT / PATCH / DELETE — GET/HEAD/OPTIONS are skipped.
///
/// Extracts user identity from the Authorization: Bearer <JWT> header.
/// Runs AFTER the request is handled so it can capture the response status.
///
/// Writes per row:
///   user_id, ip_address, method, path, status_code, duration_ms, created_at
///
/// Errors writing to audit_log are logged but do NOT fail the request.
///
/// Apply in main.rs (should be inner — auth middleware must run first):
///   .layer(axum::middleware::from_fn_with_state(
///       state.clone(),
///       middleware::audit::audit_log,
///   ))
pub async fn audit_log(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();

    // Only audit state-changing methods
    let should_audit = matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );

    if !should_audit {
        return next.run(req).await;
    }

    // Extract details before consuming the request
    let path     = req.uri().path().to_string();
    let ip       = extract_ip(&req);
    let user_id  = extract_user_id(&req, &state);

    let start = Instant::now();
    let response = next.run(req).await;
    // Cast to i32 — PostgreSQL INT is 32-bit; durations >24 days would overflow but that's acceptable.
    let duration_ms = start.elapsed().as_millis().min(i32::MAX as u128) as i32;

    let status = response.status().as_u16() as i32;

    // Fire-and-forget audit write — clone state before the async block
    let state_clone = state.clone();
    let method_str  = method.as_str().to_string();
    let path_clone  = path.clone();
    let ip_clone    = ip.clone();

    tokio::spawn(async move {
        let ip_raw = if ip_clone == "unknown" { "127.0.0.1" } else { ip_clone.as_str() };
        let ip_for_insert: sqlx::types::ipnetwork::IpNetwork = ip_raw
            .parse()
            .unwrap_or_else(|_| "127.0.0.1".parse().unwrap());

        // Retry with exponential backoff: 0ms → 200ms → 1s → give up.
        // Survives a brief DB hiccup without losing the audit row.
        let mut attempt: u32 = 0;
        let max_attempts: u32 = 3;
        loop {
            let result = sqlx::query!(
                r#"INSERT INTO audit_log
                   (user_id, ip_address, request_method, request_path,
                    status_code, duration_ms, action, target_id, created_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())"#,
                user_id, ip_for_insert, method_str, path_clone, status, duration_ms,
                format!("{} {}", method_str, path_clone),
                Option::<uuid::Uuid>::None,
            ).execute(&state_clone.db).await;

            match result {
                Ok(_)  => break,
                Err(e) => {
                    attempt += 1;
                    if attempt >= max_attempts {
                        tracing::warn!(
                            error = %e, method = %method_str, path = %path_clone,
                            attempts = attempt, "audit_log insert failed after retries"
                        );
                        // Last-resort fallback: persist to Valkey list for later flush.
                        let mut vk = state_clone.valkey.clone();
                        let payload = serde_json::json!({
                            "user_id":  user_id, "ip": ip_raw, "method": method_str,
                            "path":     path_clone, "status": status, "duration_ms": duration_ms,
                            "ts":       chrono::Utc::now().to_rfc3339(),
                        }).to_string();
                        use redis::AsyncCommands;
                        let _: Result<i64, _> = vk.lpush("orbit:audit_log_overflow", payload).await;
                        let _: Result<i64, _> = vk.ltrim::<_, ()>("orbit:audit_log_overflow", 0, 9999).await.map(|_| 0);
                        break;
                    }
                    let delay_ms = 200u64.saturating_mul(5u64.pow(attempt - 1));
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
    });

    response
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Validate that a string looks like an IP address (v4 or v6) — reject arbitrary user input.
fn looks_like_ip(s: &str) -> bool {
    // Cheap check: max 45 chars (IPv6 with zone ID), only valid IP characters
    s.len() <= 45 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '[' | ']' | '%'))
}

/// Extract the real client IP, honouring X-Forwarded-For from nginx.
///
/// SECURITY NOTE: XFF is untrusted user input. We validate that the extracted
/// value looks like a valid IP address before trusting it to prevent log injection.
fn extract_ip(req: &Request) -> String {
    // X-Forwarded-For: client, proxy1, proxy2 — take the leftmost
    if let Some(xff) = req.headers().get("x-forwarded-for") {
        if let Ok(val) = xff.to_str() {
            if let Some(first) = val.split(',').next() {
                let candidate = first.trim();
                if looks_like_ip(candidate) {
                    return candidate.to_string();
                }
            }
        }
    }

    // X-Real-IP (nginx single-IP header)
    if let Some(xri) = req.headers().get("x-real-ip") {
        if let Ok(val) = xri.to_str() {
            let candidate = val.trim();
            if looks_like_ip(candidate) {
                return candidate.to_string();
            }
        }
    }

    // ConnectInfo from Axum (direct connection — not through proxy)
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Decode JWT from the Authorization header and return the user_id if valid.
/// Returns None for unauthenticated requests (public endpoints, expired tokens).
fn extract_user_id(req: &Request, state: &Arc<AppState>) -> Option<Uuid> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?;

    let token = auth_header.strip_prefix("Bearer ")?;

    let claims = decode_jwt(token, &state.jwt_decoding_key).ok()?;
    claims.sub.parse::<Uuid>().ok()
}

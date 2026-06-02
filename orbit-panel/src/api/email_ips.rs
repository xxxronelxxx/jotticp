/// Step 3.12 — Reseller Email IP Pools
///
/// Admin / reseller endpoints for managing dedicated sending IP pools.
/// Each reseller can be assigned one or more dedicated IPs; individual
/// domains are bound to a pool entry via smtp_bind_address in Postfix.
///
/// orbit-mail daemon implements the /internal/email/ip-bind endpoints
/// that actually write the Postfix config and reload the daemon.
use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Uuid;
use std::net::IpAddr;
use tracing::{info, instrument, warn};

use crate::{AppState, ApiError, ApiResult};

// ─────────────────────────────────────────────────────────────────────────────
// DTOs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AssignIpRequest {
    reseller_id: Uuid,
    ip_address:  String,
    /// Domains to bind to this IP pool entry.
    domains:     Vec<String>,
    description: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct IpPoolRow {
    id:          Uuid,
    reseller_id: Option<Uuid>,
    ip_address:  String,
    description: Option<String>,
    created_at:  chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct IpAssignmentRow {
    id:          Uuid,
    pool_id:     Uuid,
    domain:      String,
    assigned_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct IpPoolAssignment {
    reseller_id: Option<Uuid>,
    domain:      String,
    ip_address:  String,
    status:      String,
}

#[derive(Debug, Serialize)]
struct IpReputation {
    ip_address:      String,
    clean:           bool,
    blacklists_found: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns true when the caller is an admin or the specified reseller.
fn caller_can_manage_reseller(caller: &crate::api::auth::Claims, reseller_id: Uuid) -> bool {
    caller.role == "admin"
        || caller.sub.parse::<Uuid>().map_or(false, |id| id == reseller_id)
}

/// Call orbit-mail to bind `domain` to `ip_address` for outbound SMTP.
async fn mail_ip_bind(
    state: &AppState,
    domain: &str,
    ip_address: &str,
) -> Result<(), String> {
    let url = format!("{}/internal/email/ip-bind", state.config.mail_base_url);
    let resp = state
        .http
        .post(&url)
        .json(&json!({ "domain": domain, "ip_address": ip_address }))
        .send()
        .await
        .map_err(|e| format!("mail daemon request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("mail daemon returned error: {body}"));
    }
    Ok(())
}

/// Call orbit-mail to remove the IP binding for a domain.
async fn mail_ip_unbind(
    state: &AppState,
    domain: &str,
) -> Result<(), String> {
    let url = format!(
        "{}/internal/email/ip-bind/{}",
        state.config.mail_base_url, domain
    );
    let resp = state
        .http
        .delete(&url)
        .send()
        .await
        .map_err(|e| format!("mail daemon request failed: {e}"))?;

    if !resp.status().is_success() && resp.status() != StatusCode::NOT_FOUND {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("mail daemon returned error: {body}"));
    }
    Ok(())
}

/// Check an IP against Spamhaus ZEN via DNS lookup.
/// Returns true if the IP appears in the DNSBL (i.e. it's blacklisted).
async fn check_dnsbl(ip: &str, dnsbl: &str) -> bool {
    let addr: IpAddr = match ip.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };

    // Reverse the octets for IPv4 DNSBL lookup
    let lookup_name = match addr {
        IpAddr::V4(v4) => {
            let octs = v4.octets();
            format!("{}.{}.{}.{}.{}", octs[3], octs[2], octs[1], octs[0], dnsbl)
        }
        IpAddr::V6(_) => return false, // v6 DNSBL not supported here
    };

    // Perform a simple DNS A-record lookup — listed = exists
    tokio::net::lookup_host(format!("{}:80", lookup_name))
        .await
        .is_ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/email-ips
///
/// Admin: returns all IP pool assignments across all resellers.
/// Reseller: returns assignments for their own domains only.
#[instrument(skip(state))]
async fn list_ip_pools(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let rows: Vec<IpPoolAssignment> = if caller.role == "admin" {
        sqlx::query_as!(
            IpPoolAssignment,
            r#"
            SELECT p.reseller_id,
                   a.domain,
                   p.ip_address::TEXT AS "ip_address!",
                   'active'::TEXT     AS "status!"
            FROM   email_ip_pools       p
            JOIN   email_ip_assignments a ON a.pool_id = p.id
            ORDER  BY p.created_at DESC, a.domain
            "#,
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("ip pool query failed: {e}")))?
    } else {
        // Reseller: only their own reseller_id
        sqlx::query_as!(
            IpPoolAssignment,
            r#"
            SELECT p.reseller_id,
                   a.domain,
                   p.ip_address::TEXT AS "ip_address!",
                   'active'::TEXT     AS "status!"
            FROM   email_ip_pools       p
            JOIN   email_ip_assignments a ON a.pool_id = p.id
            WHERE  p.reseller_id = $1
            ORDER  BY a.domain
            "#,
            caller_id,
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("ip pool query failed: {e}")))?
    };

    Ok(Json(json!({ "assignments": rows })))
}

/// POST /api/v1/email-ips
///
/// Assign a dedicated IP to a reseller's domains.
/// - Creates or reuses an email_ip_pools row for (reseller_id, ip_address).
/// - For each domain: calls orbit-mail to set smtp_bind_address.
/// - Inserts into email_ip_assignments.
#[instrument(skip(state, body))]
async fn assign_ip(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Json(body): Json<AssignIpRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if caller.role != "admin" && !caller_can_manage_reseller(&caller, body.reseller_id) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "forbidden"));
    }

    // Validate IP
    let _ip: IpAddr = body
        .ip_address
        .parse()
        .map_err(|_| ApiError::bad_request(format!("invalid IP address: {}", body.ip_address)))?;

    if body.domains.is_empty() {
        return Err(ApiError::bad_request("at least one domain is required"));
    }

    let ip_str: String = body.ip_address.clone();

    // Upsert pool row
    let pool_id: Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO email_ip_pools (reseller_id, ip_address, description)
        VALUES ($1, $2::text::inet, $3)
        ON CONFLICT DO NOTHING
        RETURNING id
        "#,
        body.reseller_id,
        ip_str,
        body.description,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("pool upsert failed: {e}")))?
    .unwrap_or_else(|| {
        // Row already exists — fetch its id
        Uuid::nil() // placeholder; we'll re-query below if nil
    });

    // If upsert returned nothing (conflict), fetch the existing row
    let pool_id = if pool_id == Uuid::nil() {
        sqlx::query_scalar!(
            "SELECT id FROM email_ip_pools WHERE reseller_id = $1 AND ip_address = $2::text::inet",
            body.reseller_id,
            ip_str,
        )
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("pool fetch failed: {e}")))?
    } else {
        pool_id
    };

    let mut bound_domains: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for domain in &body.domains {
        let domain = domain.to_lowercase();

        // Call orbit-mail to bind the domain to this IP
        if let Err(e) = mail_ip_bind(&state, &domain, &body.ip_address).await {
            warn!(domain = %domain, ip = %body.ip_address, error = %e, "ip-bind failed");
            errors.push(format!("{domain}: {e}"));
            continue;
        }

        // Insert or ignore if already assigned
        sqlx::query!(
            r#"
            INSERT INTO email_ip_assignments (pool_id, domain)
            VALUES ($1, $2)
            ON CONFLICT (domain) DO UPDATE SET pool_id = EXCLUDED.pool_id
            "#,
            pool_id,
            domain,
        )
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("assignment insert failed: {e}")))?;

        bound_domains.push(domain);
    }

    info!(
        pool_id = %pool_id,
        ip = %body.ip_address,
        bound = bound_domains.len(),
        errors = errors.len(),
        "IP pool assignment complete",
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "pool_id":       pool_id,
            "ip_address":    body.ip_address,
            "bound_domains": bound_domains,
            "errors":        errors,
        })),
    ))
}

/// DELETE /api/v1/email-ips/{id}
///
/// Remove an IP pool assignment (by assignment id, not pool id).
/// Calls orbit-mail to remove the smtp_bind_address config.
#[instrument(skip(state))]
async fn remove_ip_assignment(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
    Path(assignment_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    // Fetch assignment + pool to check ownership
    let row = sqlx::query!(
        r#"
        SELECT a.id, a.domain, a.pool_id,
               p.reseller_id, p.ip_address::TEXT AS ip_address
        FROM   email_ip_assignments a
        JOIN   email_ip_pools       p ON p.id = a.pool_id
        WHERE  a.id = $1
        "#,
        assignment_id,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("assignment query failed: {e}")))?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "IP assignment not found"))?;

    if caller.role != "admin" {
        let reseller_id = row
            .reseller_id
            .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "forbidden"))?;
        if !caller_can_manage_reseller(&caller, reseller_id) {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "forbidden"));
        }
    }

    // Remove binding from orbit-mail
    if let Err(e) = mail_ip_unbind(&state, &row.domain).await {
        warn!(domain = %row.domain, error = %e, "ip-unbind failed (proceeding with DB delete)");
    }

    // Delete assignment row
    sqlx::query!("DELETE FROM email_ip_assignments WHERE id = $1", assignment_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("assignment delete failed: {e}")))?;

    info!(assignment_id = %assignment_id, domain = %row.domain, "IP assignment removed");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/email-ips/reputation
///
/// For each IP in the caller's pools (admin: all pools), check against
/// Spamhaus ZEN and Barracuda BRBL DNSBLs.
#[instrument(skip(state))]
async fn check_reputation(
    State(state): State<Arc<AppState>>,
    caller: crate::api::auth::Claims,
) -> ApiResult<Json<Value>> {
    let caller_id: Uuid = caller.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let ips: Vec<String> = if caller.role == "admin" {
        sqlx::query_scalar!("SELECT ip_address::TEXT FROM email_ip_pools")
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("pool query failed: {e}")))?
            .into_iter()
            .flatten()
            .collect()
    } else {
        sqlx::query_scalar!(
            "SELECT ip_address::TEXT FROM email_ip_pools WHERE reseller_id = $1",
            caller_id,
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("pool query failed: {e}")))?
        .into_iter()
        .flatten()
        .collect()
    };

    // Check each IP concurrently against known DNSBLs
    let dnsbl_list = ["zen.spamhaus.org", "b.barracudacentral.org"];
    let mut results: Vec<IpReputation> = Vec::with_capacity(ips.len());

    for ip in ips {
        let mut blacklists_found: Vec<String> = Vec::new();
        for dnsbl in &dnsbl_list {
            if check_dnsbl(&ip, dnsbl).await {
                blacklists_found.push(dnsbl.to_string());
            }
        }
        let clean = blacklists_found.is_empty();
        results.push(IpReputation {
            ip_address: ip,
            clean,
            blacklists_found,
        });
    }

    info!(checked = results.len(), "IP reputation check complete");
    Ok(Json(json!({ "results": results })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/email/ip-pools",            get(list_ip_pools).post(assign_ip))
        .route("/api/v1/email/ip-pools/reputation", get(check_reputation))
        .route("/api/v1/email/ip-pools/{id}",       delete(remove_ip_assignment))
}

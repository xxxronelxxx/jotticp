use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// urlencoding is used by email-auth handler to percent-encode domain names in orbit-mail URL
#[allow(unused_imports)]
use urlencoding;

// ── Types ─────────────────────────────────────────────────────────────────────

const VALID_RECORD_TYPES: &[&str] = &["A", "AAAA", "MX", "TXT", "CNAME", "NS", "SRV", "CAA", "PTR"];

#[derive(Debug, Deserialize)]
pub struct CreateZoneRequest {
    pub domain: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateZoneRequest {
    pub ttl: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ZoneResponse {
    pub id:         Uuid,
    #[serde(rename = "zone")]
    pub domain:     String,
    pub site_id:    Option<Uuid>,
    pub serial:     i64,
    pub status:     String,
    pub ttl:        i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRecordRequest {
    #[serde(alias = "type")]
    pub record_type: String,
    pub name:        String,
    pub content:     String,
    pub ttl:         Option<i32>,
    pub priority:    Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRecordRequest {
    pub content:  Option<String>,
    pub ttl:      Option<i32>,
    pub priority: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct RecordResponse {
    pub id:          Uuid,
    pub zone_id:     Uuid,
    #[serde(rename = "type")]
    pub record_type: String,
    pub name:        String,
    pub content:     String,
    pub ttl:         i32,
    pub priority:    Option<i32>,
    pub disabled:    bool,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ImportZoneRequest {
    pub zone_file: String,
}

#[derive(Debug, Serialize)]
pub struct PropagationResult {
    pub nameserver:  String,
    pub record_type: String,
    pub resolved:    bool,
    pub value:       Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/dns/zones",                                    get(list_zones).post(create_zone))
        .route("/api/v1/dns/zones/{id}",                                get(get_zone).put(update_zone).delete(delete_zone))
        .route("/api/v1/dns/zones/{id}/records",                        get(list_records).post(create_record))
        .route("/api/v1/dns/zones/{id}/records/{record_id}",             put(update_record).delete(delete_record))
        .route("/api/v1/dns/zones/{id}/check-propagation",              post(check_propagation))
        .route("/api/v1/dns/zones/{id}/import",                         post(import_zone))
        // Step 3.10 — SPF/DKIM/DMARC auto-setup (canonical path)
        .route("/api/v1/dns/zones/{id}/email-auth",                     post(setup_email_auth_full))
        // Legacy alias
        .route("/api/v1/dns/zones/{id}/setup-email-auth",               post(setup_email_auth))
        // Step 4.17 — DNS propagation wizard
        .route("/api/v1/dns/propagation/{domain}/{record_type}",         get(propagation_wizard))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn fetch_zone_checked(state: &AppState, zone_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<(Uuid, String)> {
    let row = sqlx::query!(
        "SELECT dz.id, dz.zone AS domain FROM dns_zones dz
         WHERE dz.id = $1 AND dz.deleted_at IS NULL
           AND ($2::boolean OR dz.owner_id = $3)",
        zone_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "dns_zone" })?;
    Ok((row.id, row.domain))
}

// ── Zone handlers ─────────────────────────────────────────────────────────────

async fn list_zones(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<ZoneResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let rows = sqlx::query!(
        "SELECT id, zone AS domain, status, ttl, created_at FROM dns_zones
         WHERE deleted_at IS NULL AND ($1::boolean OR owner_id = $2)
         ORDER BY zone ASC",
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ZoneResponse {
        id:         r.id,
        domain:     r.domain,
        site_id:    None,
        serial:     0,
        status:     r.status,
        ttl:        r.ttl,
        created_at: r.created_at,
    }).collect()))
}

async fn get_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ZoneResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        "SELECT id, zone AS domain, status, ttl, created_at FROM dns_zones
         WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "dns_zone" })?;

    Ok(Json(ZoneResponse {
        id:         row.id,
        domain:     row.domain,
        site_id:    None,
        serial:     0,
        status:     row.status,
        ttl:        row.ttl,
        created_at: row.created_at,
    }))
}

async fn create_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateZoneRequest>,
) -> ApiResult<(StatusCode, Json<ZoneResponse>)> {
    let (user_id, _) = caller(&claims)?;

    let domain = req.domain.to_lowercase().trim().to_string();
    if !is_valid_domain(&domain) {
        return Err(ApiError::Validation(format!("'{}' is not a valid domain name", domain)));
    }

    // Check uniqueness
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM dns_zones WHERE zone = $1 AND deleted_at IS NULL)",
        domain
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::DomainConflict { domain });
    }

    let zone_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO dns_zones (id, zone, status, ttl, owner_id, created_at)
         VALUES ($1, $2, 'active', 3600, $3, NOW())",
        zone_id, domain, user_id
    )
    .execute(&state.db)
    .await?;

    // Insert default SOA + NS + A records via job
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_dns_zone",
            "zone_id": zone_id,
            "domain": domain,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(ZoneResponse {
        id:         zone_id,
        domain,
        site_id:    None,
        serial:     0,
        status:     "active".into(),
        ttl:        3600,
        created_at: Utc::now(),
    })))
}

async fn update_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateZoneRequest>,
) -> ApiResult<Json<ZoneResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_zone_checked(&state, id, user_id, is_admin).await?;

    if let Some(ttl) = req.ttl {
        if ttl < 60 || ttl > 86400 {
            return Err(ApiError::Validation("TTL must be between 60 and 86400".into()));
        }
        sqlx::query!("UPDATE dns_zones SET ttl = $1 WHERE id = $2", ttl, id)
            .execute(&state.db)
            .await?;
    }

    get_zone(State(state), claims, Path(id)).await
}

async fn delete_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, id, user_id, is_admin).await?;

    sqlx::query!("UPDATE dns_zones SET deleted_at = NOW(), status = 'deleting' WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "deprovision_dns_zone", "zone_id": id, "domain": domain});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

// ── Record handlers ───────────────────────────────────────────────────────────

async fn list_records(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
) -> ApiResult<Json<Vec<RecordResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        r#"SELECT id, zone_id, "type" AS record_type, name, content, ttl, priority, created_at
         FROM dns_records WHERE zone_id = $1 AND deleted_at IS NULL
         ORDER BY "type", name"#,
        zone_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| RecordResponse {
        id:          r.id,
        zone_id:     r.zone_id,
        record_type: r.record_type,
        name:        r.name,
        content:     r.content,
        ttl:         r.ttl,
        priority:    r.priority,
        disabled:    false,
        created_at:  r.created_at,
    }).collect()))
}

async fn create_record(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
    Json(req): Json<CreateRecordRequest>,
) -> ApiResult<(StatusCode, Json<RecordResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let rtype = req.record_type.to_uppercase();
    if !VALID_RECORD_TYPES.contains(&rtype.as_str()) {
        return Err(ApiError::Validation(format!("Invalid record type: {}", rtype)));
    }

    validate_record_content(&rtype, &req.content, req.priority)?;

    let ttl = req.ttl.unwrap_or(3600);
    if ttl < 60 || ttl > 86400 {
        return Err(ApiError::Validation("TTL must be between 60 and 86400".into()));
    }

    let record_id = Uuid::new_v4();
    let name = normalize_record_name(&req.name, &domain);

    sqlx::query!(
        r#"INSERT INTO dns_records (id, zone_id, "type", name, content, ttl, priority, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())"#,
        record_id, zone_id, rtype, name, req.content, ttl, req.priority
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_dns_record",
            "record_id": record_id,
            "zone_id": zone_id,
            "action": "create",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(RecordResponse {
        id:          record_id,
        zone_id,
        record_type: rtype,
        name,
        content:     req.content,
        ttl,
        priority:    req.priority,
        disabled:    false,
        created_at:  Utc::now(),
    })))
}

async fn update_record(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((zone_id, record_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRecordRequest>,
) -> ApiResult<Json<RecordResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let row = sqlx::query!(
        r#"SELECT id, "type" AS record_type FROM dns_records WHERE id = $1 AND zone_id = $2 AND deleted_at IS NULL"#,
        record_id, zone_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "dns_record" })?;

    if let Some(ref content) = req.content {
        validate_record_content(&row.record_type, content, req.priority)?;
        sqlx::query!("UPDATE dns_records SET content = $1 WHERE id = $2", content, record_id)
            .execute(&state.db).await?;
    }
    if let Some(ttl) = req.ttl {
        if ttl < 60 || ttl > 86400 {
            return Err(ApiError::Validation("TTL must be between 60 and 86400".into()));
        }
        sqlx::query!("UPDATE dns_records SET ttl = $1 WHERE id = $2", ttl, record_id)
            .execute(&state.db).await?;
    }
    if let Some(pri) = req.priority {
        sqlx::query!("UPDATE dns_records SET priority = $1 WHERE id = $2", pri, record_id)
            .execute(&state.db).await?;
    }

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "sync_dns_record", "record_id": record_id, "zone_id": zone_id, "action": "update"});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    let updated = sqlx::query!(
        r#"SELECT id, zone_id, "type" AS record_type, name, content, ttl, priority, created_at
         FROM dns_records WHERE id = $1"#,
        record_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(RecordResponse {
        id:          updated.id,
        zone_id:     updated.zone_id,
        record_type: updated.record_type,
        name:        updated.name,
        content:     updated.content,
        ttl:         updated.ttl,
        priority:    updated.priority,
        disabled:    false,
        created_at:  updated.created_at,
    }))
}

async fn delete_record(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((zone_id, record_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;
    fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let affected = sqlx::query!(
        "UPDATE dns_records SET deleted_at = NOW() WHERE id = $1 AND zone_id = $2 AND deleted_at IS NULL",
        record_id, zone_id
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound { resource: "dns_record" });
    }

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "sync_dns_record", "record_id": record_id, "zone_id": zone_id, "action": "delete"});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Propagation check ─────────────────────────────────────────────────────────

async fn check_propagation(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
) -> ApiResult<Json<Vec<PropagationResult>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let nameservers = ["8.8.8.8", "1.1.1.1"];
    let record_types = ["A", "MX", "NS"];
    let mut results = Vec::new();

    for ns in nameservers {
        for rtype in record_types {
            let (resolved, value) = resolve_via_nameserver(&domain, rtype, ns).await;
            results.push(PropagationResult {
                nameserver:  ns.to_string(),
                record_type: rtype.to_string(),
                resolved,
                value,
            });
        }
    }

    Ok(Json(results))
}

async fn resolve_via_nameserver(domain: &str, record_type: &str, nameserver: &str) -> (bool, Option<String>) {
    // Use `dig` subprocess for DNS resolution to avoid adding a heavy DNS library dependency
    let output = tokio::process::Command::new("dig")
        .args([
            &format!("@{}", nameserver),
            domain,
            record_type,
            "+short",
            "+time=3",
            "+tries=1",
        ])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if result.is_empty() {
                (false, None)
            } else {
                (true, Some(result))
            }
        }
        _ => (false, None),
    }
}

// ── Zone import (BIND format) ─────────────────────────────────────────────────

async fn import_zone(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
    Json(req): Json<ImportZoneRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    // Reject oversized zone files (1 MB limit)
    const MAX_ZONE_BYTES: usize = 1024 * 1024;
    if req.zone_file.len() > MAX_ZONE_BYTES {
        return Err(ApiError::Validation(format!(
            "Zone file exceeds maximum allowed size of {} bytes", MAX_ZONE_BYTES
        )));
    }

    let records = parse_bind_zone_file(&req.zone_file, &domain)?;
    let count = records.len();

    for rec in records {
        sqlx::query!(
            r#"INSERT INTO dns_records (id, zone_id, "type", name, content, ttl, priority, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
             ON CONFLICT DO NOTHING"#,
            Uuid::new_v4(), zone_id, rec.record_type, rec.name, rec.content, rec.ttl, rec.priority
        )
        .execute(&state.db)
        .await?;
    }

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "sync_dns_zone", "zone_id": zone_id, "domain": domain});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(Json(serde_json::json!({ "imported": count, "zone_id": zone_id })))
}

struct ParsedRecord {
    record_type: String,
    name:        String,
    content:     String,
    ttl:         i32,
    priority:    Option<i32>,
}

fn parse_bind_zone_file(zone_text: &str, domain: &str) -> ApiResult<Vec<ParsedRecord>> {
    let mut records = Vec::new();
    let mut current_ttl: i32 = 3600;

    for line in zone_text.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with(';') { continue; }
        // Skip $ORIGIN, $INCLUDE directives
        if line.starts_with('$') {
            if let Some(rest) = line.strip_prefix("$TTL ") {
                current_ttl = rest.trim().parse().unwrap_or(3600);
            }
            continue;
        }
        // Skip SOA
        if line.contains("SOA") { continue; }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 { continue; }

        // BIND format: [name] [ttl] [class] type rdata
        // or: [name] [class] [ttl] type rdata
        let mut idx = 0;
        let name = parts[idx].to_string();
        idx += 1;

        let mut ttl = current_ttl;
        if let Ok(t) = parts[idx].parse::<i32>() {
            ttl = t; idx += 1;
        }
        // Skip class (IN)
        if idx < parts.len() && (parts[idx] == "IN" || parts[idx] == "CHAOS") { idx += 1; }

        if idx >= parts.len() { continue; }
        let rtype = parts[idx].to_uppercase();
        idx += 1;

        if !VALID_RECORD_TYPES.contains(&rtype.as_str()) { continue; }

        let (content, priority) = if rtype == "MX" && idx < parts.len() {
            let pri: Option<i32> = parts[idx].parse().ok();
            let c = parts.get(idx + 1).copied().unwrap_or("").to_string();
            if pri.is_some() { (c, pri) } else {
                (parts[idx..].join(" "), None)
            }
        } else {
            (parts[idx..].join(" "), None)
        };

        let full_name = if name == "@" {
            format!("{}.", domain)
        } else if name.ends_with('.') {
            name.clone()
        } else {
            format!("{}.{}.", name, domain)
        };

        records.push(ParsedRecord { record_type: rtype, name: full_name, content, ttl, priority });
    }

    Ok(records)
}

// ── Email auth setup (SPF + DKIM + DMARC) ────────────────────────────────────

async fn setup_email_auth(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    let records_to_create = vec![
        ("TXT", format!("{}.", domain), format!("v=spf1 mx a ip4:127.0.0.1 ~all"), 3600i32, None::<i32>),
        ("TXT", format!("_dmarc.{}.", domain), format!("v=DMARC1; p=none; rua=mailto:dmarc@{}", domain), 3600, None),
        ("CNAME", format!("mail._domainkey.{}.", domain), format!("mail._domainkey.{}.", domain), 3600, None),
    ];

    let mut created = 0usize;
    for (rtype, name, content, ttl, priority) in records_to_create {
        // Don't overwrite existing records of same type+name
        let exists = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM dns_records WHERE zone_id = $1 AND "type" = $2 AND name = $3 AND deleted_at IS NULL)"#,
            zone_id, rtype, name
        )
        .fetch_one(&state.db)
        .await?
        .unwrap_or(false);

        if !exists {
            sqlx::query!(
                r#"INSERT INTO dns_records (id, zone_id, "type", name, content, ttl, priority, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())"#,
                Uuid::new_v4(), zone_id, rtype, name, content, ttl, priority
            )
            .execute(&state.db)
            .await?;
            created += 1;
        }
    }

    // Queue DKIM key generation via orbit-agent
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "generate_dkim_key",
            "zone_id": zone_id,
            "domain": domain,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "records_created": created,
        "message": "SPF and DMARC records created. DKIM key is being generated — check back in a few seconds.",
        "zone_id": zone_id,
    }))))
}

// ── Step 3.10 — Full SPF/DKIM/DMARC auto-setup ────────────────────────────────
//
// This is the canonical `POST /api/v1/dns/zones/{id}/email-auth` handler.
// It:
//   1. Creates/updates SPF TXT at zone apex
//   2. Calls orbit-mail to generate a DKIM keypair
//   3. Creates the DKIM TXT record (mail._domainkey.DOMAIN)
//   4. Creates the DMARC TXT record (_dmarc.DOMAIN)
//   5. Returns all four values in a structured response.

#[derive(Debug, Serialize)]
struct EmailAuthResponse {
    spf_record:    Option<String>,
    dkim_selector: String,
    dkim_record:   Option<String>,
    dmarc_record:  Option<String>,
    records_created: usize,
    dkim_status:   String,
}

async fn setup_email_auth_full(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(zone_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<EmailAuthResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let (_, domain) = fetch_zone_checked(&state, zone_id, user_id, is_admin).await?;

    // --- 1. SPF record at zone apex ---
    let spf_content = "v=spf1 a mx ~all".to_string();
    let spf_name    = format!("{}.", domain);

    let spf_created = upsert_dns_record(
        &state, zone_id, "TXT", &spf_name, &spf_content, 3600, None
    ).await?;

    // --- 2. Call orbit-mail to generate DKIM keypair ---
    let mail_url = format!(
        "{}/internal/email/domains/{}/dkim",
        state.config.orbit_mail_url,
        urlencoding::encode(&domain)
    );

    let (dkim_dns_value, dkim_status) = match state.http.post(&mail_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    let val = body.get("dns_value")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    (val, "generated".to_string())
                }
                Err(e) => {
                    tracing::warn!(error = %e, "orbit-mail DKIM response parse failed");
                    (None, "parse_error".to_string())
                }
            }
        }
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), "orbit-mail DKIM generation failed");
            (None, format!("orbit_mail_error_{}", resp.status().as_u16()))
        }
        Err(e) => {
            tracing::warn!(error = %e, "orbit-mail DKIM call failed (will queue async)");
            // Fall back to queuing via Valkey job (non-fatal)
            {
                use redis::AsyncCommands;
                let job = serde_json::json!({
                    "type": "generate_dkim_key",
                    "zone_id": zone_id,
                    "domain": domain,
                });
                let mut conn = state.valkey.clone();
                let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
            }
            (None, "queued".to_string())
        }
    };

    // --- 3. DKIM TXT record ---
    let dkim_name    = format!("mail._domainkey.{}.", domain);
    let dkim_content = dkim_dns_value.clone().unwrap_or_else(|| {
        // Placeholder while key is being generated asynchronously
        "v=DKIM1; k=ed25519; p=PENDING_KEY_GENERATION".to_string()
    });
    let dkim_created = upsert_dns_record(
        &state, zone_id, "TXT", &dkim_name, &dkim_content, 3600, None
    ).await?;

    // --- 4. DMARC TXT record ---
    let dmarc_name    = format!("_dmarc.{}.", domain);
    let dmarc_content = format!("v=DMARC1; p=none; rua=mailto:dmarc@{}", domain);
    let dmarc_created = upsert_dns_record(
        &state, zone_id, "TXT", &dmarc_name, &dmarc_content, 3600, None
    ).await?;

    let records_created = spf_created + dkim_created + dmarc_created;

    // Enqueue zone sync
    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_dns_zone",
            "zone_id": zone_id,
            "domain": domain,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(EmailAuthResponse {
        spf_record:      Some(spf_content),
        dkim_selector:   "mail".to_string(),
        dkim_record:     dkim_dns_value,
        dmarc_record:    Some(dmarc_content),
        records_created,
        dkim_status,
    })))
}

/// Insert or update a DNS record by (zone_id, record_type, name).
/// Returns 1 if a new record was inserted, 0 if an existing one was updated.
async fn upsert_dns_record(
    state:       &AppState,
    zone_id:     Uuid,
    record_type: &str,
    name:        &str,
    content:     &str,
    ttl:         i32,
    priority:    Option<i32>,
) -> ApiResult<usize> {
    let existing = sqlx::query!(
        r#"SELECT id FROM dns_records WHERE zone_id = $1 AND "type" = $2 AND name = $3 AND deleted_at IS NULL"#,
        zone_id, record_type, name
    )
    .fetch_optional(&state.db)
    .await?;

    if let Some(row) = existing {
        sqlx::query!(
            "UPDATE dns_records SET content = $1, ttl = $2 WHERE id = $3",
            content, ttl, row.id
        )
        .execute(&state.db)
        .await?;
        Ok(0)
    } else {
        sqlx::query!(
            r#"INSERT INTO dns_records (id, zone_id, "type", name, content, ttl, priority, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())"#,
            Uuid::new_v4(), zone_id, record_type, name, content, ttl, priority
        )
        .execute(&state.db)
        .await?;
        Ok(1)
    }
}

// ── Step 4.17 — DNS Propagation Wizard ────────────────────────────────────────
//
// GET /api/v1/dns/propagation/{domain}/{record_type}
// Queries three public resolvers (8.8.8.8, 1.1.1.1, 9.9.9.9) in parallel
// and returns per-resolver status.

#[derive(Debug, Serialize)]
struct ResolverResult {
    resolver: String,
    ip:       String,
    status:   String,   // "resolved" | "nxdomain" | "timeout" | "error"
    value:    Option<String>,
}

#[derive(Debug, Serialize)]
struct PropagationWizardResponse {
    domain:      String,
    record_type: String,
    resolvers:   Vec<ResolverResult>,
}

async fn propagation_wizard(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((domain, record_type)): Path<(String, String)>,
) -> ApiResult<Json<PropagationWizardResponse>> {
    // Auth: any authenticated user may query propagation
    let _ = caller(&claims)?;

    let rtype = record_type.to_uppercase();
    if !VALID_RECORD_TYPES.contains(&rtype.as_str()) {
        return Err(ApiError::Validation(format!("Invalid record type: {}", rtype)));
    }
    // Validate domain name before passing to dig subprocess
    if !is_valid_domain(&domain) {
        return Err(ApiError::Validation(format!("'{}' is not a valid domain name", domain)));
    }

    // Query three authoritative/public resolvers in parallel
    let resolvers: &[(&str, &str)] = &[
        ("8.8.8.8",   "Google"),
        ("1.1.1.1",   "Cloudflare"),
        ("9.9.9.9",   "Quad9"),
    ];

    let mut handles = Vec::new();
    for (ip, _name) in resolvers {
        let domain   = domain.clone();
        let rtype    = rtype.clone();
        let resolver_ip = ip.to_string();
        handles.push(tokio::spawn(async move {
            query_resolver(&domain, &rtype, &resolver_ip).await
        }));
    }

    let mut results = Vec::new();
    for ((ip, _name), handle) in resolvers.iter().zip(handles) {
        let (status, value) = handle.await.unwrap_or((false, None));
        results.push(ResolverResult {
            resolver: ip.to_string(),
            ip:       ip.to_string(),
            status:   if status { "resolved".into() } else { "nxdomain".into() },
            value,
        });
    }

    Ok(Json(PropagationWizardResponse {
        domain,
        record_type: rtype,
        resolvers: results,
    }))
}

/// Query a single DNS resolver via `dig` subprocess.
/// Returns (resolved: bool, value: Option<String>).
async fn query_resolver(domain: &str, record_type: &str, nameserver: &str) -> (bool, Option<String>) {
    let output = tokio::process::Command::new("dig")
        .args([
            &format!("@{}", nameserver),
            domain,
            record_type,
            "+short",
            "+time=3",
            "+tries=1",
            "+norecurse",
        ])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                (false, None)
            } else {
                (true, Some(s))
            }
        }
        _ => (false, None),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_domain(domain: &str) -> bool {
    let re = regex::Regex::new(
        r"^(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$"
    ).unwrap();
    re.is_match(domain) && domain.len() <= 253
}

fn normalize_record_name(name: &str, domain: &str) -> String {
    if name == "@" || name.is_empty() {
        return format!("{}.", domain);
    }
    if name.ends_with('.') {
        return name.to_string();
    }
    format!("{}.{}.", name, domain)
}

fn is_valid_hostname(host: &str) -> bool {
    // Strip trailing dot (fully-qualified)
    let h = host.strip_suffix('.').unwrap_or(host);
    if h.is_empty() || h.len() > 253 {
        return false;
    }
    h.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn validate_record_content(record_type: &str, content: &str, priority: Option<i32>) -> ApiResult<()> {
    if content.is_empty() {
        return Err(ApiError::Validation("Record content cannot be empty".into()));
    }

    match record_type {
        "A" => {
            if content.parse::<std::net::Ipv4Addr>().is_err() {
                return Err(ApiError::Validation(format!("Invalid IPv4 address: {}", content)));
            }
        }
        "AAAA" => {
            if content.parse::<std::net::Ipv6Addr>().is_err() {
                return Err(ApiError::Validation(format!("Invalid IPv6 address: {}", content)));
            }
        }
        "MX" => {
            if priority.is_none() {
                return Err(ApiError::Validation("MX records require a priority".into()));
            }
            if !is_valid_hostname(content) {
                return Err(ApiError::Validation(format!("Invalid MX hostname: {}", content)));
            }
        }
        "CNAME" => {
            if !is_valid_hostname(content) {
                return Err(ApiError::Validation(format!("Invalid CNAME target hostname: {}", content)));
            }
        }
        "NS" => {
            if !is_valid_hostname(content) {
                return Err(ApiError::Validation(format!("Invalid NS hostname: {}", content)));
            }
        }
        "TXT" => {
            // RFC 1035: each string in a TXT record can be 255 bytes; practical limit 4096 chars
            if content.len() > 4096 {
                return Err(ApiError::Validation(
                    "TXT record content exceeds 4096 character limit".into()
                ));
            }
            // Reject non-printable ASCII control characters (allow tab and common unicode)
            if content.chars().any(|c| c.is_ascii_control() && c != '\t') {
                return Err(ApiError::Validation(
                    "TXT record content contains invalid control characters".into()
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

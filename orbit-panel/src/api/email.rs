use axum::{
    Router,
    routing::{delete, get, post, put},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::{Claims, hash_password};

#[allow(unused_imports)]
use urlencoding;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SiteQuery {
    pub site_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EmailDomainResponse {
    pub id:      Uuid,
    pub site_id: Uuid,
    pub domain:  String,
    pub status:  String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDomainRequest {
    pub domain:  Option<String>,
    pub site_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    #[serde(alias = "domain_id")]
    pub site_id:   Uuid,
    #[serde(alias = "local_part", alias = "username")]
    pub address:   String,    // local part only — domain comes from site
    pub password:  String,
    pub quota_mb:  Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccountRequest {
    pub quota_mb: Option<i32>,
    pub enabled:  Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AccountResponse {
    pub id:          Uuid,
    pub site_id:     Option<Uuid>,
    pub domain_id:   Option<Uuid>,
    pub address:     Option<String>,
    pub local_part:  Option<String>,
    pub quota_mb:    i32,
    pub used_mb:     i32,
    pub status:      String,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateForwarderRequest {
    pub site_id:     Uuid,
    pub source:      String,  // full address or @domain for catch-all
    pub destination: String,  // external or internal address
}

#[derive(Debug, Serialize)]
pub struct ForwarderResponse {
    pub id:          Uuid,
    pub site_id:     Option<Uuid>,
    pub source:      String,
    pub destination: String,
    pub created_at:  DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAutoresponderRequest {
    pub site_id:    Uuid,
    pub account_id: Uuid,
    pub subject:    String,
    pub body:       String,
    pub start_at:   Option<DateTime<Utc>>,
    pub end_at:     Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAutoresponderRequest {
    pub subject:  Option<String>,
    pub body:     Option<String>,
    pub start_at: Option<DateTime<Utc>>,
    pub end_at:   Option<DateTime<Utc>>,
    pub enabled:  Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AutoresponderResponse {
    pub id:         Uuid,
    pub account_id: Uuid,
    pub subject:    String,
    pub body:       String,
    pub start_at:   Option<DateTime<Utc>>,
    pub end_at:     Option<DateTime<Utc>>,
    pub enabled:    bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub account_id: Uuid,
    pub address:    String,
    pub used_mb:    i64,
    pub quota_mb:   i32,
    pub pct_used:   f64,
}

#[derive(Debug, Deserialize)]
pub struct SpamThresholdRequest {
    pub score: f64,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/email/domains",                           get(list_domains).post(create_domain))
        .route("/api/v1/email/accounts",                          get(list_accounts).post(create_account))
        .route("/api/v1/email/accounts/{id}",                      get(get_account).put(update_account).delete(delete_account))
        .route("/api/v1/email/accounts/{id}/password",             post(change_password))
        .route("/api/v1/email/accounts/{id}/spam-threshold",       post(set_spam_threshold))
        .route("/api/v1/email/forwarders",                        get(list_forwarders).post(create_forwarder))
        .route("/api/v1/email/forwarders/{id}",                    delete(delete_forwarder))
        .route("/api/v1/email/autoresponders",                    get(list_autoresponders).post(create_autoresponder))
        .route("/api/v1/email/autoresponders/{id}",                put(update_autoresponder).delete(delete_autoresponder))
        .route("/api/v1/email/usage",                             get(get_usage))
        // Step 3.15 — Sieve filter proxy
        .route("/api/v1/email/mailboxes/{id}/sieve",               get(get_sieve).put(put_sieve).delete(delete_sieve))
        // Step 3.16 — Smarthost proxy
        .route("/api/v1/email/domains/{domain}/smarthost",         post(set_smarthost).delete(delete_smarthost))
        // Step 3.17 — Mail queue proxy
        .route("/api/v1/email/queue",                             get(get_queue))
        .route("/api/v1/email/queue/flush-all",                   post(flush_all_queue))
        .route("/api/v1/email/queue/{queue_id}/flush",             post(flush_queue_message))
        .route("/api/v1/email/queue/{queue_id}",                   delete(delete_queue_message))
        // Step 3.18 — Roundcube SSO token
        .route("/api/v1/email/mailboxes/{id}/webmail-sso",         post(webmail_sso))
        // Step 3.19 — SMTP sending limits
        .route("/api/v1/email/smtp-limits",                        get(list_smtp_limits))
        .route("/api/v1/email/smtp-limits/{domain}",               get(get_smtp_limits).put(upsert_smtp_limits))
        .route("/api/v1/email/smtp-limits/{domain}/reset",         post(reset_smtp_counters))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

async fn assert_site_owner(state: &AppState, site_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<String> {
    let site = sqlx::query!(
        "SELECT domain FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
        site_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "site" })?;
    Ok(site.domain)
}

// ── Email account handlers ────────────────────────────────────────────────────

async fn list_domains(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<EmailDomainResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let rows = sqlx::query!(
        "SELECT id, domain FROM sites WHERE deleted_at IS NULL AND ($1::boolean OR owner_id = $2) ORDER BY domain ASC",
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| EmailDomainResponse {
        id:      r.id,
        site_id: r.id,
        domain:  r.domain,
        status:  "active".to_string(),
    }).collect()))
}

async fn create_domain(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateDomainRequest>,
) -> ApiResult<(StatusCode, Json<EmailDomainResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;

    // Resolve site_id + domain from the request
    let (site_id, domain) = match (req.site_id, req.domain.as_deref()) {
        (Some(sid), _) => {
            let site = sqlx::query!(
                "SELECT id, domain FROM sites WHERE id = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
                sid, is_admin, user_id
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound { resource: "site" })?;
            (site.id, site.domain)
        }
        (None, Some(dom)) => {
            let site = sqlx::query!(
                "SELECT id, domain FROM sites WHERE domain = $1 AND deleted_at IS NULL AND ($2::boolean OR owner_id = $3)",
                dom, is_admin, user_id
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound { resource: "site" })?;
            (site.id, site.domain)
        }
        (None, None) => return Err(ApiError::Validation("site_id or domain is required".into())),
    };

    // Upsert email_domains record for this site
    let rec = sqlx::query!(
        r#"INSERT INTO email_domains (id, domain, site_id, status, created_at)
           VALUES (gen_random_uuid(), $1, $2, 'active', NOW())
           ON CONFLICT (domain) DO UPDATE SET site_id = EXCLUDED.site_id, status = 'active'
           RETURNING id, domain, site_id, status"#,
        domain, site_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(EmailDomainResponse {
        id:      rec.id,
        site_id: rec.site_id.unwrap_or(site_id),
        domain:  rec.domain,
        status:  rec.status,
    })))
}

async fn list_accounts(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<SiteQuery>,
) -> ApiResult<Json<Vec<AccountResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    let accounts: Vec<AccountResponse> = if let Some(site_id) = q.site_id {
        assert_site_owner(&state, site_id, user_id, is_admin).await?;
        sqlx::query!(
            "SELECT id, site_id, address, quota_mb, used_mb, status, created_at
             FROM email_accounts WHERE site_id = $1 AND deleted_at IS NULL
             ORDER BY address ASC",
            site_id
        )
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|r| AccountResponse {
            id:         r.id,
            site_id:    r.site_id,
            domain_id:  r.site_id,
            address:    r.address.clone(),
            local_part: r.address.as_deref().and_then(|a| a.split('@').next()).map(String::from),
            quota_mb:   r.quota_mb,
            used_mb:    r.used_mb,
            status:     r.status,
            created_at: r.created_at,
        })
        .collect()
    } else {
        sqlx::query!(
            r#"SELECT ea.id, ea.site_id, ea.address, ea.quota_mb, ea.used_mb, ea.status, ea.created_at
               FROM email_accounts ea
               JOIN sites s ON s.id = ea.site_id
               WHERE ea.deleted_at IS NULL AND s.deleted_at IS NULL
                 AND ($1::boolean OR s.owner_id = $2)
               ORDER BY ea.address ASC"#,
            is_admin, user_id
        )
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|r| AccountResponse {
            id:         r.id,
            site_id:    r.site_id,
            domain_id:  r.site_id,
            address:    r.address.clone(),
            local_part: r.address.as_deref().and_then(|a| a.split('@').next()).map(String::from),
            quota_mb:   r.quota_mb,
            used_mb:    r.used_mb,
            status:     r.status,
            created_at: r.created_at,
        })
        .collect()
    };

    Ok(Json(accounts))
}

async fn get_account(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AccountResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT ea.id, ea.site_id, ea.address, ea.quota_mb, ea.used_mb, ea.status, ea.created_at
           FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    Ok(Json(AccountResponse {
        id:         row.id,
        site_id:    row.site_id,
        domain_id:  row.site_id,
        address:    row.address.clone(),
        local_part: row.address.as_deref().and_then(|a| a.split('@').next()).map(String::from),
        quota_mb:   row.quota_mb,
        used_mb:    row.used_mb,
        status:     row.status,
        created_at: row.created_at,
    }))
}

async fn create_account(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateAccountRequest>,
) -> ApiResult<(StatusCode, Json<AccountResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let domain = assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    if req.password.len() < 10 {
        return Err(ApiError::Validation("Password must be at least 10 characters".into()));
    }
    if !is_strong_password(&req.password) {
        return Err(ApiError::Validation(
            "Password must contain at least one uppercase letter, one lowercase letter, one digit, and one special character".into()
        ));
    }

    // Validate local part
    let local = req.address.to_lowercase();
    if !is_valid_email_local(&local) {
        return Err(ApiError::Validation("Invalid email local part".into()));
    }

    let full_address = format!("{}@{}", local, domain);

    // Check uniqueness
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM email_accounts WHERE address = $1 AND deleted_at IS NULL)",
        full_address
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if exists {
        return Err(ApiError::DomainConflict { domain: full_address.clone() });
    }

    let account_id = Uuid::new_v4();
    let quota_mb = req.quota_mb.unwrap_or(1024);

    // Enforce quota ceiling (10 GB)
    if quota_mb < 1 || quota_mb > 10240 {
        return Err(ApiError::Validation("quota_mb must be between 1 and 10240 (10 GB)".into()));
    }

    // Get or create email_domain record (required by email_accounts FK)
    let domain_id: Uuid = {
        let existing = sqlx::query_scalar!(
            "SELECT id FROM email_domains WHERE domain = $1",
            domain
        )
        .fetch_optional(&state.db)
        .await?;

        match existing {
            Some(id) => id,
            None => {
                let new_id = Uuid::new_v4();
                sqlx::query!(
                    "INSERT INTO email_domains (id, domain, site_id, status, created_at) VALUES ($1, $2, $3, 'active', NOW())",
                    new_id, domain, req.site_id
                )
                .execute(&state.db)
                .await?;
                new_id
            }
        }
    };

    let password_hash = hash_password(&req.password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("password hash failed: {}", e)))?;

    sqlx::query!(
        "INSERT INTO email_accounts (id, domain_id, site_id, local_part, address, password_hash, quota_mb, used_mb, status, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'provisioning', NOW())",
        account_id, domain_id, req.site_id, local, full_address, password_hash, quota_mb
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "provision_email_account",
            "account_id": account_id,
            "site_id": req.site_id,
            "address": full_address.clone(),
            "password": req.password,
            "quota_mb": quota_mb,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
        // Cache cleartext credential for webmail SSO auto-login (30-day TTL survives restarts)
        let cred_key = format!("orbit:mail:credential:{}", account_id);
        let _: () = conn.set_ex(cred_key, &req.password, 30 * 86400).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(AccountResponse {
        id:         account_id,
        site_id:    Some(req.site_id),
        domain_id:  Some(req.site_id),
        address:    Some(full_address),
        local_part: Some(local),
        quota_mb,
        used_mb:    0,
        status:     "provisioning".into(),
        created_at: Utc::now(),
    })))
}

async fn update_account(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAccountRequest>,
) -> ApiResult<Json<AccountResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Ownership check via join
    let _ = sqlx::query!(
        r#"SELECT ea.id FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    if let Some(quota) = req.quota_mb {
        sqlx::query!(
            "UPDATE email_accounts SET quota_mb = $1 WHERE id = $2",
            quota, id
        )
        .execute(&state.db)
        .await?;

        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "update_email_quota", "account_id": id, "quota_mb": quota});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    if let Some(enabled) = req.enabled {
        let status = if enabled { "active" } else { "suspended" };
        sqlx::query!("UPDATE email_accounts SET status = $1 WHERE id = $2", status, id)
            .execute(&state.db)
            .await?;
    }

    get_account(State(state), claims, Path(id)).await
}

async fn delete_account(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT ea.id, ea.address, ea.site_id FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    sqlx::query!("UPDATE email_accounts SET deleted_at = NOW(), status = 'deleting' WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "deprovision_email_account",
            "account_id": row.id,
            "address": row.address,
            "site_id": row.site_id,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

async fn change_password(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    if req.password.len() < 10 {
        return Err(ApiError::Validation("Password must be at least 10 characters".into()));
    }
    if !is_strong_password(&req.password) {
        return Err(ApiError::Validation(
            "Password must contain at least one uppercase letter, one lowercase letter, one digit, and one special character".into()
        ));
    }

    let row = sqlx::query!(
        r#"SELECT ea.id, ea.address FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "change_email_password",
            "account_id": row.id,
            "address": row.address,
            "new_password": req.password,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
        // Update cached cleartext credential for webmail SSO (30-day TTL survives restarts)
        let cred_key = format!("orbit:mail:credential:{}", id);
        let _: () = conn.set_ex(cred_key, &req.password, 30 * 86400).await.unwrap_or(());
    }

    Ok(StatusCode::ACCEPTED)
}

async fn set_spam_threshold(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<SpamThresholdRequest>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    if req.score < 1.0 || req.score > 15.0 {
        return Err(ApiError::Validation("Spam threshold score must be between 1.0 and 15.0".into()));
    }

    let row = sqlx::query!(
        r#"SELECT ea.id, ea.address FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    sqlx::query!(
        "UPDATE email_accounts SET spam_threshold = $1::float8::numeric WHERE id = $2",
        req.score, id
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "set_spam_threshold",
            "account_id": row.id,
            "address": row.address,
            "score": req.score,
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::OK)
}

// ── Forwarder handlers ────────────────────────────────────────────────────────

async fn list_forwarders(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<SiteQuery>,
) -> ApiResult<Json<Vec<ForwarderResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let site_id = q.site_id.ok_or_else(|| ApiError::Validation("site_id is required".into()))?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        "SELECT id, site_id, source, destination, created_at
         FROM email_forwarders WHERE site_id = $1 AND deleted_at IS NULL
         ORDER BY source ASC",
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| ForwarderResponse {
        id:          r.id,
        site_id:     r.site_id,
        source:      r.source.unwrap_or_default(),
        destination: r.destination.unwrap_or_default(),
        created_at:  r.created_at,
    }).collect()))
}

async fn create_forwarder(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateForwarderRequest>,
) -> ApiResult<(StatusCode, Json<ForwarderResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    let domain = assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    // Source must belong to this domain
    if !req.source.ends_with(&format!("@{}", domain)) && req.source != format!("@{}", domain) {
        return Err(ApiError::Validation(
            format!("Forwarder source must be an address at @{} or the catch-all @{}", domain, domain)
        ));
    }

    // Loop detection: destination must not point back to source domain's MX
    if req.destination.ends_with(&format!("@{}", domain)) {
        return Err(ApiError::Validation("Forwarding to the same domain creates a loop".into()));
    }

    let fwd_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO email_forwarders (id, site_id, source, destination, created_at)
         VALUES ($1, $2, $3, $4, NOW())",
        fwd_id, req.site_id, req.source, req.destination
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_email_forwarder",
            "forwarder_id": fwd_id,
            "site_id": req.site_id,
            "source": req.source,
            "destination": req.destination,
            "action": "create",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(ForwarderResponse {
        id:          fwd_id,
        site_id:     Some(req.site_id),
        source:      req.source,
        destination: req.destination,
        created_at:  Utc::now(),
    })))
}

async fn delete_forwarder(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT ef.id, ef.source, ef.destination, ef.site_id FROM email_forwarders ef
           JOIN sites s ON s.id = ef.site_id
           WHERE ef.id = $1 AND ef.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "forwarder" })?;

    sqlx::query!("UPDATE email_forwarders SET deleted_at = NOW() WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_email_forwarder",
            "forwarder_id": row.id,
            "site_id": row.site_id,
            "source": row.source.unwrap_or_default(),
            "destination": row.destination.unwrap_or_default(),
            "action": "delete",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Autoresponder handlers ────────────────────────────────────────────────────

async fn list_autoresponders(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<SiteQuery>,
) -> ApiResult<Json<Vec<AutoresponderResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let site_id = q.site_id.ok_or_else(|| ApiError::Validation("site_id is required".into()))?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        r#"SELECT ar.id, ar.account_id, ar.subject, ar.body,
                  ar.start_at, ar.end_at, ar.enabled, ar.created_at
           FROM email_autoresponders ar
           JOIN email_accounts ea ON ea.id = ar.account_id
           WHERE ea.site_id = $1 AND ar.deleted_at IS NULL
           ORDER BY ar.created_at DESC"#,
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| AutoresponderResponse {
        id:         r.id,
        account_id: r.account_id,
        subject:    r.subject,
        body:       r.body,
        start_at:   r.start_at,
        end_at:     r.end_at,
        enabled:    r.enabled,
        created_at: r.created_at,
    }).collect()))
}

async fn create_autoresponder(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<CreateAutoresponderRequest>,
) -> ApiResult<(StatusCode, Json<AutoresponderResponse>)> {
    let (user_id, is_admin) = caller(&claims)?;
    assert_site_owner(&state, req.site_id, user_id, is_admin).await?;

    // Verify account belongs to this site
    let _ = sqlx::query!(
        "SELECT id FROM email_accounts WHERE id = $1 AND site_id = $2 AND deleted_at IS NULL",
        req.account_id, req.site_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;

    if req.subject.is_empty() {
        return Err(ApiError::Validation("Subject is required".into()));
    }
    if req.subject.len() > 998 {
        return Err(ApiError::Validation("Subject must not exceed 998 characters (RFC 5321)".into()));
    }

    // Validate date range: if both present, end must be after start
    if let (Some(start), Some(end)) = (req.start_at, req.end_at) {
        if end <= start {
            return Err(ApiError::Validation("end_at must be after start_at".into()));
        }
    }

    let ar_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO email_autoresponders (id, account_id, subject, body, start_at, end_at, enabled, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, true, NOW())",
        ar_id, req.account_id, req.subject, req.body, req.start_at, req.end_at
    )
    .execute(&state.db)
    .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({
            "type": "sync_autoresponder",
            "autoresponder_id": ar_id,
            "account_id": req.account_id,
            "action": "create",
        });
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok((StatusCode::CREATED, Json(AutoresponderResponse {
        id:         ar_id,
        account_id: req.account_id,
        subject:    req.subject,
        body:       req.body,
        start_at:   req.start_at,
        end_at:     req.end_at,
        enabled:    true,
        created_at: Utc::now(),
    })))
}

async fn update_autoresponder(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAutoresponderRequest>,
) -> ApiResult<Json<AutoresponderResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Fetch existing record for ownership check and date cross-validation
    let existing = sqlx::query!(
        r#"SELECT ar.id, ar.start_at, ar.end_at FROM email_autoresponders ar
           JOIN email_accounts ea ON ea.id = ar.account_id
           JOIN sites s ON s.id = ea.site_id
           WHERE ar.id = $1 AND ar.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "autoresponder" })?;

    // Cross-validate dates: compute what start/end will be after the update
    let effective_start = req.start_at.or(existing.start_at);
    let effective_end   = req.end_at.or(existing.end_at);
    if let (Some(start), Some(end)) = (effective_start, effective_end) {
        if end <= start {
            return Err(ApiError::Validation("end_at must be after start_at".into()));
        }
    }

    if let Some(ref subject) = req.subject {
        sqlx::query!("UPDATE email_autoresponders SET subject = $1 WHERE id = $2", subject, id)
            .execute(&state.db).await?;
    }
    if let Some(ref body) = req.body {
        sqlx::query!("UPDATE email_autoresponders SET body = $1 WHERE id = $2", body, id)
            .execute(&state.db).await?;
    }
    if let Some(start) = req.start_at {
        sqlx::query!("UPDATE email_autoresponders SET start_at = $1 WHERE id = $2", start, id)
            .execute(&state.db).await?;
    }
    if let Some(end) = req.end_at {
        sqlx::query!("UPDATE email_autoresponders SET end_at = $1 WHERE id = $2", end, id)
            .execute(&state.db).await?;
    }
    if let Some(enabled) = req.enabled {
        sqlx::query!("UPDATE email_autoresponders SET enabled = $1 WHERE id = $2", enabled, id)
            .execute(&state.db).await?;
    }

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "sync_autoresponder", "autoresponder_id": id, "action": "update"});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    let row = sqlx::query!(
        "SELECT id, account_id, subject, body, start_at, end_at, enabled, created_at
         FROM email_autoresponders WHERE id = $1",
        id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AutoresponderResponse {
        id:         row.id,
        account_id: row.account_id,
        subject:    row.subject,
        body:       row.body,
        start_at:   row.start_at,
        end_at:     row.end_at,
        enabled:    row.enabled,
        created_at: row.created_at,
    }))
}

async fn delete_autoresponder(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (user_id, is_admin) = caller(&claims)?;

    let _ = sqlx::query!(
        r#"SELECT ar.id FROM email_autoresponders ar
           JOIN email_accounts ea ON ea.id = ar.account_id
           JOIN sites s ON s.id = ea.site_id
           WHERE ar.id = $1 AND ar.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "autoresponder" })?;

    sqlx::query!("UPDATE email_autoresponders SET deleted_at = NOW(), enabled = false WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    {
        use redis::AsyncCommands;
        let job = serde_json::json!({"type": "sync_autoresponder", "autoresponder_id": id, "action": "delete"});
        let mut conn = state.valkey.clone();
        let _: () = conn.lpush("orbit:jobs", job.to_string()).await.unwrap_or(());
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn get_usage(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<SiteQuery>,
) -> ApiResult<Json<Vec<UsageResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let site_id = q.site_id.ok_or_else(|| ApiError::Validation("site_id is required".into()))?;
    assert_site_owner(&state, site_id, user_id, is_admin).await?;

    let rows = sqlx::query!(
        "SELECT id, address, used_mb, quota_mb FROM email_accounts
         WHERE site_id = $1 AND deleted_at IS NULL ORDER BY address",
        site_id
    )
    .fetch_all(&state.db)
    .await?;

    let usage = rows.into_iter().map(|r| {
        let quota = r.quota_mb as f64;
        let used = r.used_mb as f64;
        let pct = if quota > 0.0 { (used / quota * 100.0).min(100.0) } else { 0.0 };
        UsageResponse {
            account_id: r.id,
            address:    r.address.unwrap_or_default(),
            used_mb:    r.used_mb as i64,
            quota_mb:   r.quota_mb,
            pct_used:   pct,
        }
    }).collect();

    Ok(Json(usage))
}

// ── Step 3.19 — SMTP Sending Limits ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SmtpLimitsRequest {
    pub hourly_limit:  Option<i32>,
    pub daily_limit:   Option<i32>,
    pub rate_action:   Option<String>,
    pub relay_enabled: Option<bool>,
    pub require_spf:   Option<bool>,
    pub require_dkim:  Option<bool>,
    pub enabled:       Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SmtpLimitsResponse {
    pub id:             Uuid,
    pub site_id:        Option<Uuid>,
    pub domain:         String,
    pub hourly_limit:   i32,
    pub daily_limit:    i32,
    pub rate_action:    String,
    pub relay_enabled:  bool,
    pub require_spf:    bool,
    pub require_dkim:   bool,
    pub enabled:        bool,
    pub sent_this_hour: i64,
    pub sent_today:     i64,
}

async fn list_smtp_limits(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<Vec<SmtpLimitsResponse>>> {
    let (user_id, is_admin) = caller(&claims)?;

    // Admins see all; regular users see only limits for their own sites
    let rows = sqlx::query!(
        r#"SELECT
               l.id, l.site_id, l.domain,
               l.hourly_limit, l.daily_limit, l.rate_action,
               l.relay_enabled, l.require_spf, l.require_dkim, l.enabled,
               COALESCE(c.sent_count, 0)::bigint AS sent_this_hour,
               COALESCE(c.day_count,  0)::bigint AS sent_today
           FROM smtp_domain_limits l
           LEFT JOIN smtp_send_counts c
               ON c.domain = l.domain
              AND c.hour_bucket = date_trunc('hour', NOW())
           WHERE ($1::boolean
               OR EXISTS (
                   SELECT 1 FROM sites s
                   WHERE s.id = l.site_id
                     AND s.owner_id = $2
                     AND s.deleted_at IS NULL
               ))
           ORDER BY l.domain ASC"#,
        is_admin, user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| SmtpLimitsResponse {
        id:             r.id,
        site_id:        r.site_id,
        domain:         r.domain,
        hourly_limit:   r.hourly_limit,
        daily_limit:    r.daily_limit,
        rate_action:    r.rate_action,
        relay_enabled:  r.relay_enabled,
        require_spf:    r.require_spf,
        require_dkim:   r.require_dkim,
        enabled:        r.enabled,
        sent_this_hour: r.sent_this_hour.unwrap_or(0),
        sent_today:     r.sent_today.unwrap_or(0),
    }).collect()))
}

async fn get_smtp_limits(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(domain): Path<String>,
) -> ApiResult<Json<SmtpLimitsResponse>> {
    let (user_id, is_admin) = caller(&claims)?;

    let row = sqlx::query!(
        r#"SELECT
               l.id, l.site_id, l.domain,
               l.hourly_limit, l.daily_limit, l.rate_action,
               l.relay_enabled, l.require_spf, l.require_dkim, l.enabled,
               COALESCE(c.sent_count, 0)::bigint AS sent_this_hour,
               COALESCE(c.day_count,  0)::bigint AS sent_today
           FROM smtp_domain_limits l
           LEFT JOIN smtp_send_counts c
               ON c.domain = l.domain
              AND c.hour_bucket = date_trunc('hour', NOW())
           WHERE l.domain = $1
             AND ($2::boolean
               OR EXISTS (
                   SELECT 1 FROM sites s
                   WHERE s.id = l.site_id
                     AND s.owner_id = $3
                     AND s.deleted_at IS NULL
               ))"#,
        domain, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "smtp_limits" })?;

    Ok(Json(SmtpLimitsResponse {
        id:             row.id,
        site_id:        row.site_id,
        domain:         row.domain,
        hourly_limit:   row.hourly_limit,
        daily_limit:    row.daily_limit,
        rate_action:    row.rate_action,
        relay_enabled:  row.relay_enabled,
        require_spf:    row.require_spf,
        require_dkim:   row.require_dkim,
        enabled:        row.enabled,
        sent_this_hour: row.sent_this_hour.unwrap_or(0),
        sent_today:     row.sent_today.unwrap_or(0),
    }))
}

async fn upsert_smtp_limits(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(domain): Path<String>,
    Json(req): Json<SmtpLimitsRequest>,
) -> ApiResult<Json<SmtpLimitsResponse>> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }

    // Validate rate_action if provided
    if let Some(ref action) = req.rate_action {
        if !matches!(action.as_str(), "queue" | "reject" | "bounce") {
            return Err(ApiError::Validation("rate_action must be queue, reject, or bounce".into()));
        }
    }

    // Validate limits if provided
    if let Some(h) = req.hourly_limit {
        if h < 10 || h > 50000 {
            return Err(ApiError::Validation("hourly_limit must be between 10 and 50000".into()));
        }
    }
    if let Some(d) = req.daily_limit {
        if d < 10 || d > 1000000 {
            return Err(ApiError::Validation("daily_limit must be between 10 and 1000000".into()));
        }
    }

    // Cross-field validation: daily_limit must be >= hourly_limit when both
    // are provided in the same request (non-zero values represent real limits).
    if let (Some(h), Some(d)) = (req.hourly_limit, req.daily_limit) {
        if h > 0 && d > 0 && d < h {
            return Err(ApiError::Validation(
                "daily_limit must be greater than or equal to hourly_limit".into(),
            ));
        }
    }

    // Try to find a site_id that matches this domain
    let site_id: Option<Uuid> = sqlx::query_scalar!(
        "SELECT id FROM sites WHERE domain = $1 AND deleted_at IS NULL LIMIT 1",
        domain
    )
    .fetch_optional(&state.db)
    .await?;

    sqlx::query!(
        r#"INSERT INTO smtp_domain_limits
               (domain, site_id, hourly_limit, daily_limit, rate_action,
                relay_enabled, require_spf, require_dkim, enabled, updated_at)
           VALUES ($1, $2,
               COALESCE($3, 500), COALESCE($4, 5000), COALESCE($5, 'queue'),
               COALESCE($6, true), COALESCE($7, false), COALESCE($8, false),
               COALESCE($9, true), NOW())
           ON CONFLICT (domain) DO UPDATE SET
               site_id       = COALESCE($2,  smtp_domain_limits.site_id),
               hourly_limit  = COALESCE($3,  smtp_domain_limits.hourly_limit),
               daily_limit   = COALESCE($4,  smtp_domain_limits.daily_limit),
               rate_action   = COALESCE($5,  smtp_domain_limits.rate_action),
               relay_enabled = COALESCE($6,  smtp_domain_limits.relay_enabled),
               require_spf   = COALESCE($7,  smtp_domain_limits.require_spf),
               require_dkim  = COALESCE($8,  smtp_domain_limits.require_dkim),
               enabled       = COALESCE($9,  smtp_domain_limits.enabled),
               updated_at    = NOW()"#,
        domain,
        site_id,
        req.hourly_limit,
        req.daily_limit,
        req.rate_action,
        req.relay_enabled,
        req.require_spf,
        req.require_dkim,
        req.enabled,
    )
    .execute(&state.db)
    .await?;

    get_smtp_limits(State(state), claims, Path(domain)).await
}

async fn reset_smtp_counters(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(domain): Path<String>,
) -> ApiResult<StatusCode> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Forbidden("Admin access required".into()));
    }

    // Delete current hour bucket and today's day bucket entries for this domain
    sqlx::query!(
        r#"DELETE FROM smtp_send_counts
           WHERE domain = $1
             AND (hour_bucket = date_trunc('hour', NOW())
               OR day_bucket  = CURRENT_DATE)"#,
        domain
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_email_local(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.contains('@')
        && local.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Require at least one uppercase, one lowercase, one digit, one special char.
fn is_strong_password(pw: &str) -> bool {
    let has_upper   = pw.chars().any(|c| c.is_ascii_uppercase());
    let has_lower   = pw.chars().any(|c| c.is_ascii_lowercase());
    let has_digit   = pw.chars().any(|c| c.is_ascii_digit());
    let has_special = pw.chars().any(|c| !c.is_alphanumeric());
    has_upper && has_lower && has_digit && has_special
}

// ── Step 3.15 — Sieve filter proxy ───────────────────────────────────────────

/// Resolve the email address for a mailbox account id.
async fn resolve_email_address(state: &AppState, id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<String> {
    let row = sqlx::query!(
        r#"SELECT ea.address FROM email_accounts ea
           JOIN sites s ON s.id = ea.site_id
           WHERE ea.id = $1 AND ea.deleted_at IS NULL
             AND s.deleted_at IS NULL AND ($2::boolean OR s.owner_id = $3)"#,
        id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "email_account" })?;
    Ok(row.address.unwrap_or_default())
}

async fn get_sieve(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Response> {
    let (user_id, is_admin) = caller(&claims)?;
    let address = resolve_email_address(&state, id, user_id, is_admin).await?;

    let url = format!("{}/internal/email/mailboxes/{}/sieve", state.config.orbit_mail_url, urlencoding::encode(&address));
    proxy_get(&state, &url).await
}

async fn put_sieve(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
    body: axum::body::Bytes,
) -> ApiResult<axum::response::Response> {
    let (user_id, is_admin) = caller(&claims)?;
    let address = resolve_email_address(&state, id, user_id, is_admin).await?;

    // Enforce 64 KB max for Sieve scripts
    const MAX_SIEVE_BYTES: usize = 64 * 1024;
    if body.len() > MAX_SIEVE_BYTES {
        return Err(ApiError::Validation(format!(
            "Sieve script exceeds maximum allowed size of {} bytes", MAX_SIEVE_BYTES
        )));
    }

    // Validate UTF-8 — Sieve scripts must be text; binary blobs are rejected
    if std::str::from_utf8(&body).is_err() {
        return Err(ApiError::Validation("Sieve script must be valid UTF-8 text".into()));
    }

    let url = format!("{}/internal/email/mailboxes/{}/sieve", state.config.orbit_mail_url, urlencoding::encode(&address));
    proxy_put(&state, &url, body).await
}

async fn delete_sieve(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Response> {
    let (user_id, is_admin) = caller(&claims)?;
    let address = resolve_email_address(&state, id, user_id, is_admin).await?;

    let url = format!("{}/internal/email/mailboxes/{}/sieve", state.config.orbit_mail_url, urlencoding::encode(&address));
    proxy_delete(&state, &url).await
}

// ── Step 3.16 — Smarthost proxy ───────────────────────────────────────────────

async fn set_smarthost(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(domain): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<axum::response::Response> {
    let (_, is_admin) = caller(&claims)?;
    // Only admins can configure smarthost relay
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    let url = format!("{}/internal/email/domains/{}/smarthost", state.config.orbit_mail_url, urlencoding::encode(&domain));
    // orbit-mail uses POST for smarthost creation/update
    proxy_post_body(&state, &url, body).await
}

async fn delete_smarthost(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(domain): Path<String>,
) -> ApiResult<axum::response::Response> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    let url = format!("{}/internal/email/domains/{}/smarthost", state.config.orbit_mail_url, urlencoding::encode(&domain));
    proxy_delete(&state, &url).await
}

// ── Step 3.17 — Mail queue proxy ─────────────────────────────────────────────

async fn get_queue(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    let url = format!("{}/internal/email/queue", state.config.orbit_mail_url);

    // Degrade gracefully when orbit-mail sidecar is not deployed: return
    // an empty queue so the UI shows "0 messages" rather than a 500.
    match state.http.get(&url).send().await {
        Ok(resp) => {
            let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::OK);
            let body = resp.bytes().await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;
            Ok((status, [(CONTENT_TYPE, "application/json")], body).into_response())
        }
        Err(_) => {
            let empty = r#"{"messages":[],"total":0,"orbit_mail_available":false}"#;
            Ok((StatusCode::OK, [(CONTENT_TYPE, "application/json")], empty).into_response())
        }
    }
}

async fn flush_all_queue(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<axum::response::Response> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    let url = format!("{}/internal/email/queue/flush-all", state.config.orbit_mail_url);
    proxy_post_empty(&state, &url).await
}

async fn flush_queue_message(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(queue_id): Path<String>,
) -> ApiResult<axum::response::Response> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    // Validate queue_id — only alphanumeric to prevent injection
    if !queue_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::Validation("invalid queue_id".into()));
    }
    let url = format!("{}/internal/email/queue/{}/flush", state.config.orbit_mail_url, queue_id);
    proxy_post_empty(&state, &url).await
}

async fn delete_queue_message(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(queue_id): Path<String>,
) -> ApiResult<axum::response::Response> {
    let (_, is_admin) = caller(&claims)?;
    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    if !queue_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::Validation("invalid queue_id".into()));
    }
    let url = format!("{}/internal/email/queue/{}", state.config.orbit_mail_url, queue_id);
    proxy_delete(&state, &url).await
}

// ── Step 3.18 — Roundcube SSO token ──────────────────────────────────────────

#[derive(Debug, Serialize)]
struct WebmailSsoResponse {
    sso_url:   String,
    token:     String,
    expires_in: u64,
}

async fn webmail_sso(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WebmailSsoResponse>> {
    let (user_id, is_admin) = caller(&claims)?;
    let address = resolve_email_address(&state, id, user_id, is_admin).await?;

    // Generate a 16-byte random token, hex-encoded → 32 hex chars
    let token_bytes: [u8; 16] = rand::random();
    let token = hex::encode(token_bytes);

    // Extract domain from email address
    let domain = address
        .split('@')
        .nth(1)
        .unwrap_or("localhost")
        .to_string();

    let expires_in: u64 = 300; // 5 minutes
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + expires_in;

    // Look up cached cleartext credential for auto-login
    let mail_password: Option<String> = {
        use redis::AsyncCommands;
        let cred_key = format!("orbit:mail:credential:{}", id);
        let mut conn = state.valkey.clone();
        conn.get(cred_key).await.unwrap_or(None)
    };

    // Store in Valkey: orbit:webmail:TOKEN = JSON payload with TTL
    {
        use redis::AsyncCommands;
        let key = format!("orbit:webmail:{token}");
        let payload = serde_json::json!({
            "email":    address,
            "expires":  expires_at,
            "password": mail_password,
        });
        let mut conn = state.valkey.clone();
        // SET with EX (seconds TTL)
        let _: () = conn
            .set_ex(key, payload.to_string(), expires_in)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("valkey set: {e}")))?;
    }

    let sso_url = format!("{}?token={}", state.config.webmail_url.trim_end_matches('/'), token);

    Ok(Json(WebmailSsoResponse {
        sso_url,
        token,
        expires_in,
    }))
}

// ── Internal proxy helpers ────────────────────────────────────────────────────
//
// These forward requests to orbit-mail at state.config.orbit_mail_url.
// They pass the raw body through and return the orbit-mail JSON response.

async fn proxy_get(state: &AppState, url: &str) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let resp = state.http.get(url).send().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail GET: {e}")))?;

    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = resp.bytes().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;

    Ok((status, [(CONTENT_TYPE, "application/json")], body).into_response())
}

async fn proxy_put(state: &AppState, url: &str, body: axum::body::Bytes) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let resp = state.http.put(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail PUT: {e}")))?;

    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_body = resp.bytes().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;

    Ok((status, [(CONTENT_TYPE, "application/json")], resp_body).into_response())
}

async fn proxy_post_body(state: &AppState, url: &str, body: axum::body::Bytes) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let resp = state.http.post(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail POST: {e}")))?;

    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_body = resp.bytes().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;

    Ok((status, [(CONTENT_TYPE, "application/json")], resp_body).into_response())
}

async fn proxy_post_empty(state: &AppState, url: &str) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let resp = state.http.post(url).send().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail POST: {e}")))?;

    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_body = resp.bytes().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;

    Ok((status, [(CONTENT_TYPE, "application/json")], resp_body).into_response())
}

async fn proxy_delete(state: &AppState, url: &str) -> ApiResult<axum::response::Response> {
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    let resp = state.http.delete(url).send().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail DELETE: {e}")))?;

    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_body = resp.bytes().await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("orbit-mail body: {e}")))?;

    Ok((status, [(CONTENT_TYPE, "application/json")], resp_body).into_response())
}
